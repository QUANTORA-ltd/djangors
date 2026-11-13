#![deny(missing_docs)]
//! Pure-Rust SAML 2.0 Service Provider (SP) single sign-on for Djangors.
//!
//! Includes:
//! - Strict pure-Rust XML-DSig verification (`xml-exc-c14n#` canonicalization +
//!   `RSA-SHA256` / `RSA-SHA512` / `ECDSA-P256-SHA256`) with zero C dependencies
//!   (`libxml2` / `xmlsec1` not required).
//! - Anti-XXE (`<!DOCTYPE` / entity rejection) and anti-XSW structural checks
//!   (unique `ID`s, sole direct-child `<Assertion>`, never accepts unsigned assertions).
//! - [`SamlIdentityProvider`] model (holds IdP metadata XML and X.509 certificates,
//!   editable in admin via [`register_saml_admin`]).
//! - SP metadata (`/saml/{idp}/metadata`), `/saml/{idp}/login` (`AuthnRequest` over
//!   HTTP-Redirect), `/saml/{idp}/acs` (`Response`/`Assertion` verification + replay
//!   protection via `djangors_cache::Cache` + attribute mapping + JIT user creation).

/// Error types for SAML 2.0 operations.
pub mod error;
pub(crate) mod xmldsig;

pub use error::SamlError;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::{DateTime, Utc};
use djangors_auth::{login, User};
use djangors_cache::Cache;
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_orm::{q, Model as _};
use djangors_sessions::Session;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use hyper::http::HeaderValue;
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
use xmldsig::{
    parse_idp_certificates, parse_xml, verify_saml_response_signatures, IdpVerifyingKey,
    XmlElement, XMLNS_DSIG, XMLNS_SAML_ASSERTION, XMLNS_SAML_PROTOCOL,
};

const XMLNS_SAML_METADATA: &str = "urn:oasis:names:tc:SAML:2.0:metadata";
const SAML_STATUS_SUCCESS: &str = "urn:oasis:names:tc:SAML:2.0:status:Success";
const SAML_BINDING_HTTP_REDIRECT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
const SAML_BINDING_HTTP_POST: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";
const SESSION_PENDING_PREFIX: &str = "_saml_pending_";
const DEFAULT_PENDING_TTL_SECS: i64 = 600;
const DEFAULT_MAX_CLOCK_SKEW_SECS: i64 = 60;

/// Service Provider (SP) configuration inserted into [`djangors_core::AppState`].
#[derive(Debug, Clone)]
pub struct SamlSpConfig {
    /// SP EntityID (`<Audience>` expected inside incoming assertions).
    pub entity_id: String,
    /// Public base URL of this application (e.g. `"https://app.example.com"`), no trailing slash.
    pub base_url: String,
    /// Optional SP X.509 PEM certificate advertised in SP metadata.
    pub sp_x509_cert_pem: Option<String>,
    /// Whether unsolicited IdP-initiated responses without a matching `InResponseTo` are allowed.
    /// Defaults to `false` (strict SP-initiated validation).
    pub allow_idp_initiated: bool,
    /// Allowed clock skew in seconds for `NotBefore` / `NotOnOrAfter` checks (default: 60s).
    pub max_clock_skew_secs: i64,
    /// Fallback replay-cache TTL when an assertion has no `NotOnOrAfter` delta (default: 3600s).
    pub default_replay_ttl: Duration,
    /// Safe relative redirect path after successful SSO login (default: `"/"`).
    pub default_redirect_after_login: String,
}

impl SamlSpConfig {
    /// Creates a new SAML Service Provider configuration.
    pub fn new(entity_id: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            sp_x509_cert_pem: None,
            allow_idp_initiated: false,
            max_clock_skew_secs: DEFAULT_MAX_CLOCK_SKEW_SECS,
            default_replay_ttl: Duration::from_secs(3600),
            default_redirect_after_login: "/".to_string(),
        }
    }

    /// Sets the SP X.509 PEM certificate included in SP metadata.
    pub fn sp_x509_cert_pem(mut self, pem: impl Into<String>) -> Self {
        self.sp_x509_cert_pem = Some(pem.into());
        self
    }

    /// Configures whether unsolicited IdP-initiated assertions (`InResponseTo` absent) are accepted.
    pub fn allow_idp_initiated(mut self, allow: bool) -> Self {
        self.allow_idp_initiated = allow;
        self
    }

    /// Sets allowed clock skew in seconds for assertion timestamp validation.
    pub fn max_clock_skew_secs(mut self, secs: i64) -> Self {
        self.max_clock_skew_secs = secs.max(0);
        self
    }

    /// Sets the fallback post-login redirect path.
    pub fn default_redirect_after_login(mut self, path: impl Into<String>) -> Self {
        self.default_redirect_after_login = path.into();
        self
    }

    /// Returns the ACS URL for `idp_slug`: `{base_url}/saml/{idp_slug}/acs`.
    pub fn acs_url(&self, idp_slug: &str) -> String {
        format!("{}/saml/{idp_slug}/acs", self.base_url)
    }

    /// Returns the SP metadata URL for `idp_slug`: `{base_url}/saml/{idp_slug}/metadata`.
    pub fn metadata_url(&self, idp_slug: &str) -> String {
        format!("{}/saml/{idp_slug}/metadata", self.base_url)
    }
}

/// Persisted SAML 2.0 Identity Provider configuration, editable in admin.
#[derive(Model, Debug, Clone)]
#[djangors(app = "saml", table_name = "saml_identity_provider")]
pub struct SamlIdentityProvider {
    /// Primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// URL slug identifying this IdP in `/saml/{idp}/login` and `/saml/{idp}/acs`.
    #[djangors(max_length = 64, unique)]
    pub slug: String,
    /// Human-readable display name (e.g. `"Okta Corp"`).
    #[djangors(max_length = 150)]
    pub name: String,
    /// IdP EntityID URI.
    #[djangors(max_length = 500)]
    pub entity_id: String,
    /// IdP HTTP-Redirect SingleSignOnService URL.
    #[djangors(max_length = 1000)]
    pub sso_url: String,
    /// Pinned X.509 signing certificate(s) in PEM or raw base64 form.
    pub x509_cert_pem: String,
    /// Optional raw IdP SAML 2.0 `EntityDescriptor` metadata XML.
    pub metadata_xml: String,
    /// JSON-serialized [`SamlAttributeMap`] controlling attribute-to-user mapping.
    pub attribute_map_json: String,
    /// Whether unknown SAML users should be provisioned just-in-time (`password = "!"`).
    pub jit_create_users: bool,
    /// Whether this IdP configuration is enabled.
    pub is_active: bool,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
}

impl SamlIdentityProvider {
    /// Constructs a [`SamlIdentityProvider`] directly from IdP metadata XML.
    ///
    /// Extracts `entityID`, HTTP-Redirect `SingleSignOnService`, and all
    /// `<X509Certificate>` entries from the `EntityDescriptor`.
    pub fn from_metadata_xml(
        slug: impl Into<String>,
        name: impl Into<String>,
        metadata_xml: impl Into<String>,
    ) -> Result<Self, SamlError> {
        let xml = metadata_xml.into();
        let parsed = parse_idp_metadata(&xml)?;
        Ok(Self {
            id: 0,
            slug: slug.into(),
            name: name.into(),
            entity_id: parsed.entity_id,
            sso_url: parsed.sso_url,
            x509_cert_pem: parsed.x509_certs_pem,
            metadata_xml: xml,
            attribute_map_json: "{}".to_string(),
            jit_create_users: true,
            is_active: true,
            created_at: Utc::now(),
        })
    }

    /// Parses and validates the pinned IdP signing certificates for this provider.
    pub(crate) fn verifying_certificates(&self) -> Result<Vec<IdpVerifyingKey>, SamlError> {
        if !self.x509_cert_pem.trim().is_empty() {
            return parse_idp_certificates(&self.x509_cert_pem);
        }
        if !self.metadata_xml.trim().is_empty() {
            let parsed = parse_idp_metadata(&self.metadata_xml)?;
            return parse_idp_certificates(&parsed.x509_certs_pem);
        }
        Err(SamlError::InvalidCertificate(format!(
            "SAML IdP '{}' has neither x509_cert_pem nor metadata_xml certificates",
            self.slug
        )))
    }

    /// Returns the decoded [`SamlAttributeMap`] for this provider.
    pub fn attribute_map(&self) -> SamlAttributeMap {
        if self.attribute_map_json.trim().is_empty() {
            SamlAttributeMap::default()
        } else {
            serde_json::from_str(&self.attribute_map_json).unwrap_or_default()
        }
    }
}

/// Creates the `saml_identity_provider` table using dialect-appropriate SQL.
///
/// Call once at application startup or in migrations/tests (never per request).
pub async fn create_saml_tables(db: &Database) -> Result<(), djangors_orm::OrmError> {
    let mut conn = db.conn();
    let d = conn.dialect();
    let pk = d.auto_pk_type();
    let b = d.bool_type();
    let ts = d.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS saml_identity_provider (
            id {pk},
            slug VARCHAR(64) NOT NULL UNIQUE,
            name VARCHAR(150) NOT NULL,
            entity_id VARCHAR(500) NOT NULL,
            sso_url VARCHAR(1000) NOT NULL,
            x509_cert_pem TEXT NOT NULL,
            metadata_xml TEXT NOT NULL,
            attribute_map_json TEXT NOT NULL,
            jit_create_users {b} NOT NULL,
            is_active {b} NOT NULL,
            created_at {ts} NOT NULL
        )"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(djangors_orm::OrmError::Query)?;
    Ok(())
}

/// Registers [`SamlIdentityProvider`] with an [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_saml_admin(site: &djangors_admin::AdminSite) {
    site.register::<SamlIdentityProvider>();
}

/// Mapping from SAML assertion attribute names to Djangors identity fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlAttributeMap {
    /// Candidate SAML attribute names mapped to `User::username`.
    #[serde(default = "default_username_attrs")]
    pub username: Vec<String>,
    /// Candidate SAML attribute names mapped to `User::email`.
    #[serde(default = "default_email_attrs")]
    pub email: Vec<String>,
    /// Candidate SAML attribute names extracted into `SamlAssertionClaims::first_name`.
    #[serde(default = "default_first_name_attrs")]
    pub first_name: Vec<String>,
    /// Candidate SAML attribute names extracted into `SamlAssertionClaims::last_name`.
    #[serde(default = "default_last_name_attrs")]
    pub last_name: Vec<String>,
}

fn default_username_attrs() -> Vec<String> {
    vec![
        "username".to_string(),
        "uid".to_string(),
        "preferred_username".to_string(),
        "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/name".to_string(),
    ]
}

fn default_email_attrs() -> Vec<String> {
    vec![
        "email".to_string(),
        "mail".to_string(),
        "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress".to_string(),
    ]
}

fn default_first_name_attrs() -> Vec<String> {
    vec![
        "first_name".to_string(),
        "givenName".to_string(),
        "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/givenname".to_string(),
    ]
}

fn default_last_name_attrs() -> Vec<String> {
    vec![
        "last_name".to_string(),
        "sn".to_string(),
        "surname".to_string(),
        "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/surname".to_string(),
    ]
}

impl Default for SamlAttributeMap {
    fn default() -> Self {
        Self {
            username: default_username_attrs(),
            email: default_email_attrs(),
            first_name: default_first_name_attrs(),
            last_name: default_last_name_attrs(),
        }
    }
}

/// Verified claims extracted from a cryptographically validated SAML 2.0 assertion.
#[derive(Debug, Clone)]
pub struct SamlAssertionClaims {
    /// Assertion `ID` attribute.
    pub assertion_id: String,
    /// Subject `<NameID>` value.
    pub name_id: String,
    /// Resolved username candidate (from mapped attributes or `<NameID>`).
    pub username: Option<String>,
    /// Resolved email address (from mapped attributes or email-formatted `<NameID>`).
    pub email: Option<String>,
    /// Optional mapped first name.
    pub first_name: Option<String>,
    /// Optional mapped last name.
    pub last_name: Option<String>,
    /// All `<Attribute>` name -> values extracted from `<AttributeStatement>`.
    pub attributes: HashMap<String, Vec<String>>,
    /// Session `InResponseTo` request ID matched by the assertion, if any.
    pub in_response_to: Option<String>,
    /// `NotOnOrAfter` timestamp observed on the assertion/confirmation, if any.
    pub not_on_or_after: Option<DateTime<Utc>>,
}

/// Returns a [`Router`] mounting the SAML 2.0 Service Provider endpoints:
///
/// - `GET /saml/{idp}/metadata`
/// - `GET /saml/{idp}/login`
/// - `POST /saml/{idp}/acs`
pub fn saml_router() -> Router {
    Router::new()
        .get("/saml/{idp}/metadata", saml_metadata_view)
        .get("/saml/{idp}/login", saml_login_view)
        .post("/saml/{idp}/acs", saml_acs_view)
}

#[derive(Serialize, Deserialize)]
struct PendingSamlRequest {
    request_id: String,
    idp_slug: String,
    redirect_to: String,
    created_at: i64,
}

/// `GET /saml/{idp}/metadata`: returns SP SAML 2.0 `EntityDescriptor` metadata XML.
pub async fn saml_metadata_view(
    req: Request,
    params: PathParams,
) -> Result<Response, DjangorsError> {
    let idp_slug = require_idp_slug(&params)?;
    let sp = require_sp_config(&req)?;
    let xml = render_sp_metadata_xml(&sp, &idp_slug);
    Ok(Response::bytes(
        StatusCode::OK,
        "application/samlmetadata+xml; charset=utf-8",
        xml.into_bytes(),
    ))
}

/// Renders the Service Provider's SAML 2.0 `EntityDescriptor` metadata XML for `idp_slug`.
pub fn render_sp_metadata_xml(sp: &SamlSpConfig, idp_slug: &str) -> String {
    let entity_id = escape_xml(&sp.entity_id);
    let acs_url = escape_xml(&sp.acs_url(idp_slug));
    let key_desc = if let Some(pem) = sp.sp_x509_cert_pem.as_deref() {
        let stripped = strip_pem_headers(pem);
        format!(
            "<md:KeyDescriptor use=\"signing\"><ds:KeyInfo xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\"><ds:X509Data><ds:X509Certificate>{}</ds:X509Certificate></ds:X509Data></ds:KeyInfo></md:KeyDescriptor>",
            escape_xml(&stripped)
        )
    } else {
        String::new()
    };

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <md:EntityDescriptor xmlns:md=\"{XMLNS_SAML_METADATA}\" entityID=\"{entity_id}\">\
           <md:SPSSODescriptor AuthnRequestsSigned=\"false\" WantAssertionsSigned=\"true\" \
             protocolSupportEnumeration=\"{XMLNS_SAML_PROTOCOL}\">\
             {key_desc}\
             <md:NameIDFormat>urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress</md:NameIDFormat>\
             <md:AssertionConsumerService Binding=\"{SAML_BINDING_HTTP_POST}\" Location=\"{acs_url}\" index=\"0\" isDefault=\"true\"/>\
           </md:SPSSODescriptor>\
         </md:EntityDescriptor>"
    )
}

/// `GET /saml/{idp}/login`: generates a SAML `AuthnRequest` and redirects to the IdP SSO URL.
pub async fn saml_login_view(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let idp_slug = require_idp_slug(&params)?;
    let db = require_db(&req)?;
    let sp = require_sp_config(&req)?;
    let session = require_session(&req)?;
    let idp = load_active_idp(db, &idp_slug).await?;

    let redirect_to = sanitize_next_url(
        extract_query_param(req.raw_query(), "next").as_deref(),
        &sp.default_redirect_after_login,
    );

    let request_id = format!("_dj_{}", random_hex_id(16));
    let relay_state = random_hex_id(24);
    let now = Utc::now();
    let pending = PendingSamlRequest {
        request_id: request_id.clone(),
        idp_slug: idp.slug.clone(),
        redirect_to,
        created_at: now.timestamp(),
    };
    let pending_key = format!("{SESSION_PENDING_PREFIX}{relay_state}");
    session.set(&pending_key, &pending);

    let authn_xml = build_authn_request_xml(&request_id, now, &sp, &idp);
    let encoded_saml_request = deflate_and_base64(&authn_xml).map_err(|e| {
        DjangorsError::Internal(format!("SAML AuthnRequest compression failed: {e}"))
    })?;

    let mut sso = Url::parse(&idp.sso_url).map_err(|e| {
        DjangorsError::Internal(format!("invalid IdP sso_url '{}': {e}", idp.sso_url))
    })?;
    {
        let mut pairs = sso.query_pairs_mut();
        pairs.append_pair("SAMLRequest", &encoded_saml_request);
        pairs.append_pair("RelayState", &relay_state);
    }

    HeaderValue::from_str(sso.as_str())
        .map_err(|e| DjangorsError::Internal(format!("invalid SAML redirect header: {e}")))?;
    Ok(Response::text(StatusCode::FOUND, "").header("location", sso.as_str()))
}

/// `POST /saml/{idp}/acs`: verifies the SAMLResponse signature, claims, and replay cache,
/// provisions or loads the local [`User`], logs them in, and redirects.
pub async fn saml_acs_view(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let idp_slug = require_idp_slug(&params)?;
    let db = require_db(&req)?;
    let sp = require_sp_config(&req)?;
    let cache = require_cache(&req)?;
    let session = require_session(&req)?;
    let idp = load_active_idp(db, &idp_slug).await?;

    let body_bytes = req.body_bytes().await;
    let form: HashMap<String, String> = url::form_urlencoded::parse(body_bytes)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let raw_saml = form
        .get("SAMLResponse")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| SamlError::InvalidMetadata("missing SAMLResponse form field".to_string()))?;

    let expected_request = match form
        .get("RelayState")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
    {
        Some(rs) => {
            let key = format!("{SESSION_PENDING_PREFIX}{rs}");
            let pending = session.get::<PendingSamlRequest>(&key);
            session.remove(&key);
            pending
        }
        None => None,
    };

    let (expected_request_id, redirect_to) = match expected_request {
        Some(p) => {
            let age = Utc::now().timestamp() - p.created_at;
            if p.idp_slug != idp.slug || age > DEFAULT_PENDING_TTL_SECS {
                return Err(SamlError::RelayStateMismatch.into());
            }
            (Some(p.request_id), p.redirect_to)
        }
        None => {
            if !sp.allow_idp_initiated {
                return Err(SamlError::RelayStateMismatch.into());
            }
            (None, sp.default_redirect_after_login.clone())
        }
    };

    let claims = verify_and_extract_saml_response(
        raw_saml,
        &idp,
        &sp,
        expected_request_id.as_deref(),
        cache.as_ref(),
    )
    .await?;

    let user = resolve_or_create_saml_user(db, &idp, &claims).await?;
    login(session, &user);

    let safe_next = sanitize_next_url(Some(&redirect_to), &sp.default_redirect_after_login);
    let safe_loc = if HeaderValue::from_str(&safe_next).is_ok() {
        safe_next.as_str()
    } else {
        "/"
    };
    Ok(Response::text(StatusCode::FOUND, "").header("location", safe_loc))
}

/// Decodes, cryptographically verifies, checks assertion conditions and replay cache,
/// and returns [`SamlAssertionClaims`].
pub async fn verify_and_extract_saml_response(
    saml_response_b64: &str,
    idp: &SamlIdentityProvider,
    sp: &SamlSpConfig,
    expected_in_response_to: Option<&str>,
    replay_cache: &dyn Cache,
) -> Result<SamlAssertionClaims, SamlError> {
    let cleaned_b64: String = saml_response_b64
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let xml_bytes = STANDARD
        .decode(&cleaned_b64)
        .map_err(|e| SamlError::InvalidMetadata(format!("invalid base64 SAMLResponse: {e}")))?;
    let xml_str = std::str::from_utf8(&xml_bytes).map_err(|e| {
        SamlError::InvalidMetadata(format!("SAMLResponse XML is not valid UTF-8: {e}"))
    })?;

    let root = parse_xml(xml_str)?;
    let pinned_keys = idp.verifying_certificates()?;
    let (response_el, assertion_el) = verify_saml_response_signatures(&root, &pinned_keys)?;

    // Check Status == Success.
    let status_code = response_el
        .find_direct_child(XMLNS_SAML_PROTOCOL, "Status")
        .and_then(|s| s.find_direct_child(XMLNS_SAML_PROTOCOL, "StatusCode"))
        .and_then(|sc| sc.attr("Value"))
        .unwrap_or("");
    if status_code != SAML_STATUS_SUCCESS {
        return Err(SamlError::StatusNotSuccess(status_code.to_string()));
    }

    let expected_acs = sp.acs_url(&idp.slug);
    if let Some(dest) = response_el.attr("Destination") {
        if dest != expected_acs {
            return Err(SamlError::RecipientMismatch {
                expected: expected_acs,
                actual: dest.to_string(),
            });
        }
    }

    if let Some(expected_req_id) = expected_in_response_to {
        let resp_irt = response_el.attr("InResponseTo");
        if resp_irt != Some(expected_req_id) {
            return Err(SamlError::InResponseToMismatch(format!(
                "Response InResponseTo {:?} != expected '{}'",
                resp_irt, expected_req_id
            )));
        }
    }

    let assertion_id = assertion_el
        .attr("ID")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SamlError::InvalidMetadata("Assertion is missing ID attribute".to_string()))?
        .to_string();

    let now = Utc::now();
    let skew = chrono::Duration::seconds(sp.max_clock_skew_secs);

    // Subject & SubjectConfirmationData checks (Recipient, NotOnOrAfter, InResponseTo).
    let subject_el = assertion_el
        .find_direct_child(XMLNS_SAML_ASSERTION, "Subject")
        .ok_or_else(|| SamlError::MissingAttribute("Subject".to_string()))?;
    let name_id = subject_el
        .find_direct_child(XMLNS_SAML_ASSERTION, "NameID")
        .map(|n| n.text_content().trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SamlError::MissingAttribute("Subject/NameID".to_string()))?;

    let bearer_confirm = subject_el
        .child_elements()
        .filter(|e| {
            e.namespace_uri == XMLNS_SAML_ASSERTION && e.local_name == "SubjectConfirmation"
        })
        .find(|sc| sc.attr("Method") == Some("urn:oasis:names:tc:SAML:2.0:cm:bearer"))
        .ok_or_else(|| {
            SamlError::InvalidMetadata("missing bearer SubjectConfirmation".to_string())
        })?;

    let sc_data = bearer_confirm
        .find_direct_child(XMLNS_SAML_ASSERTION, "SubjectConfirmationData")
        .ok_or_else(|| SamlError::InvalidMetadata("missing SubjectConfirmationData".to_string()))?;

    let recipient = sc_data.attr("Recipient").unwrap_or("");
    if recipient != expected_acs {
        return Err(SamlError::RecipientMismatch {
            expected: expected_acs,
            actual: recipient.to_string(),
        });
    }

    let sc_not_on_or_after_raw = sc_data.attr("NotOnOrAfter").ok_or_else(|| {
        SamlError::ExpiredAssertion("SubjectConfirmationData missing NotOnOrAfter".to_string())
    })?;
    let sc_not_on_or_after = parse_saml_datetime(sc_not_on_or_after_raw)?;
    if now - skew >= sc_not_on_or_after {
        return Err(SamlError::ExpiredAssertion(format!(
            "SubjectConfirmationData NotOnOrAfter {sc_not_on_or_after} exceeded"
        )));
    }
    let mut earliest_not_on_or_after = Some(sc_not_on_or_after);

    if let Some(expected_req_id) = expected_in_response_to {
        let sc_irt = sc_data.attr("InResponseTo");
        if sc_irt != Some(expected_req_id) {
            return Err(SamlError::InResponseToMismatch(format!(
                "SubjectConfirmationData InResponseTo {:?} != expected '{}'",
                sc_irt, expected_req_id
            )));
        }
    }

    // Conditions: NotBefore, NotOnOrAfter, and AudienceRestriction.
    let conditions = assertion_el
        .find_direct_child(XMLNS_SAML_ASSERTION, "Conditions")
        .ok_or_else(|| SamlError::InvalidMetadata("Assertion is missing Conditions".to_string()))?;

    if let Some(nb_raw) = conditions.attr("NotBefore") {
        let nb = parse_saml_datetime(nb_raw)?;
        if now + skew < nb {
            return Err(SamlError::ExpiredAssertion(format!(
                "Assertion NotBefore {nb} is in the future"
            )));
        }
    }
    if let Some(noa_raw) = conditions.attr("NotOnOrAfter") {
        let noa = parse_saml_datetime(noa_raw)?;
        if now - skew >= noa {
            return Err(SamlError::ExpiredAssertion(format!(
                "Assertion Conditions NotOnOrAfter {noa} exceeded"
            )));
        }
        earliest_not_on_or_after = Some(match earliest_not_on_or_after {
            Some(prev) if prev < noa => prev,
            _ => noa,
        });
    }

    let audiences: Vec<String> = conditions
        .child_elements()
        .filter(|e| {
            e.namespace_uri == XMLNS_SAML_ASSERTION && e.local_name == "AudienceRestriction"
        })
        .flat_map(|ar| ar.child_elements())
        .filter(|a| a.namespace_uri == XMLNS_SAML_ASSERTION && a.local_name == "Audience")
        .map(|a| a.text_content().trim().to_string())
        .collect();

    if !audiences.iter().any(|a| a == &sp.entity_id) {
        return Err(SamlError::AudienceMismatch {
            expected: sp.entity_id.clone(),
            actual: audiences.join(", "),
        });
    }

    // Replay check via atomic Cache::add.
    let ttl = earliest_not_on_or_after
        .and_then(|dt| (dt + skew - now).to_std().ok())
        .unwrap_or(sp.default_replay_ttl)
        .max(Duration::from_secs(60));
    let replay_key = format!("saml:replay:{}:{}", idp.slug, assertion_id);
    if !replay_cache
        .add(&replay_key, b"1".to_vec(), Some(ttl))
        .await?
    {
        return Err(SamlError::ReplayDetected(assertion_id));
    }

    // Extract attributes.
    let mut attributes: HashMap<String, Vec<String>> = HashMap::new();
    for stmt in assertion_el.child_elements() {
        if stmt.namespace_uri == XMLNS_SAML_ASSERTION && stmt.local_name == "AttributeStatement" {
            for attr_el in stmt.child_elements() {
                if attr_el.namespace_uri == XMLNS_SAML_ASSERTION
                    && attr_el.local_name == "Attribute"
                {
                    let Some(attr_name) = attr_el.attr("Name").filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    let vals: Vec<String> = attr_el
                        .child_elements()
                        .filter(|v| {
                            v.namespace_uri == XMLNS_SAML_ASSERTION
                                && v.local_name == "AttributeValue"
                        })
                        .map(|v| v.text_content().trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !vals.is_empty() {
                        attributes
                            .entry(attr_name.to_string())
                            .or_default()
                            .extend(vals);
                    }
                }
            }
        }
    }

    let attr_map = idp.attribute_map();
    let email = first_attr_value(&attributes, &attr_map.email).or_else(|| {
        if name_id.contains('@') {
            Some(name_id.clone())
        } else {
            None
        }
    });
    let username =
        first_attr_value(&attributes, &attr_map.username).or_else(|| Some(name_id.clone()));
    let first_name = first_attr_value(&attributes, &attr_map.first_name);
    let last_name = first_attr_value(&attributes, &attr_map.last_name);

    Ok(SamlAssertionClaims {
        assertion_id,
        name_id,
        username,
        email,
        first_name,
        last_name,
        attributes,
        in_response_to: sc_data.attr("InResponseTo").map(ToString::to_string),
        not_on_or_after: earliest_not_on_or_after,
    })
}

/// Resolves an existing local [`User`] or performs just-in-time user provisioning
/// according to `idp` settings.
pub async fn resolve_or_create_saml_user(
    db: &Database,
    idp: &SamlIdentityProvider,
    claims: &SamlAssertionClaims,
) -> Result<User, SamlError> {
    let raw_username = claims
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(claims.name_id.as_str());
    let normalized_email = claims
        .email
        .as_deref()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());

    if !raw_username.is_empty() {
        if let Some(user) = User::objects()
            .filter(q!(username = raw_username))?
            .first(db)
            .await?
        {
            if !user.is_active {
                return Err(SamlError::InactiveUser);
            }
            return Ok(user);
        }
    }

    if let Some(email) = normalized_email.as_deref() {
        let matches = User::objects().filter(q!(email = email))?.all(db).await?;
        if matches.len() > 1 {
            return Err(SamlError::MissingAttribute(format!(
                "multiple local users match email '{email}'"
            )));
        }
        if let Some(user) = matches.into_iter().next() {
            if !user.is_active {
                return Err(SamlError::InactiveUser);
            }
            return Ok(user);
        }
    }

    if !idp.jit_create_users {
        return Err(SamlError::MissingAttribute(
            "local user does not exist and JIT provisioning is disabled".to_string(),
        ));
    }

    let username = generate_unique_saml_username(db, raw_username, &claims.name_id).await?;
    let user = User {
        id: 0,
        username,
        email: normalized_email.unwrap_or_default(),
        password: "!".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    };
    Ok(user.save(db).await?)
}

// ---------------------------------------------------------------------------
// Helpers: Metadata parsing, AuthnRequest encoding, and request utilities
// ---------------------------------------------------------------------------

struct ParsedIdpMetadata {
    entity_id: String,
    sso_url: String,
    x509_certs_pem: String,
}

fn parse_idp_metadata(xml: &str) -> Result<ParsedIdpMetadata, SamlError> {
    let root = parse_xml(xml)?;
    if root.namespace_uri != XMLNS_SAML_METADATA || root.local_name != "EntityDescriptor" {
        return Err(SamlError::InvalidMetadata(
            "IdP metadata root must be md:EntityDescriptor".to_string(),
        ));
    }
    let entity_id = root
        .attr("entityID")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| SamlError::InvalidMetadata("EntityDescriptor missing entityID".to_string()))?
        .to_string();

    let idp_sso = root
        .find_direct_child(XMLNS_SAML_METADATA, "IDPSSODescriptor")
        .ok_or_else(|| SamlError::InvalidMetadata("missing md:IDPSSODescriptor".to_string()))?;

    let sso_url = idp_sso
        .child_elements()
        .filter(|e| e.namespace_uri == XMLNS_SAML_METADATA && e.local_name == "SingleSignOnService")
        .find(|e| e.attr("Binding") == Some(SAML_BINDING_HTTP_REDIRECT))
        .and_then(|e| e.attr("Location"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            SamlError::InvalidMetadata(
                "missing HTTP-Redirect SingleSignOnService Location".to_string(),
            )
        })?
        .to_string();

    let mut cert_blocks = Vec::new();
    for kd in idp_sso.child_elements() {
        if kd.namespace_uri == XMLNS_SAML_METADATA && kd.local_name == "KeyDescriptor" {
            let use_attr = kd.attr("use");
            if matches!(use_attr, None | Some("signing")) {
                collect_x509_certificates(kd, &mut cert_blocks);
            }
        }
    }
    if cert_blocks.is_empty() {
        return Err(SamlError::InvalidCertificate(
            "IdP metadata contains no signing X509Certificate".to_string(),
        ));
    }

    // Validate parsed certificates eagerly.
    let pem = cert_blocks.join("\n");
    let _ = parse_idp_certificates(&pem)?;

    Ok(ParsedIdpMetadata {
        entity_id,
        sso_url,
        x509_certs_pem: pem,
    })
}

fn collect_x509_certificates(node: &XmlElement, out: &mut Vec<String>) {
    if node.namespace_uri == XMLNS_DSIG && node.local_name == "X509Certificate" {
        let raw: String = node
            .text_content()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        if !raw.is_empty() {
            out.push(format!(
                "-----BEGIN CERTIFICATE-----\n{raw}\n-----END CERTIFICATE-----"
            ));
        }
    }
    for child in node.child_elements() {
        collect_x509_certificates(child, out);
    }
}

fn build_authn_request_xml(
    request_id: &str,
    now: DateTime<Utc>,
    sp: &SamlSpConfig,
    idp: &SamlIdentityProvider,
) -> String {
    let issue_instant = now.format("%Y-%m-%dT%H:%M:%SZ");
    let acs = escape_xml(&sp.acs_url(&idp.slug));
    let dest = escape_xml(&idp.sso_url);
    let issuer = escape_xml(&sp.entity_id);
    let id = escape_xml(request_id);
    format!(
        "<samlp:AuthnRequest xmlns:samlp=\"{XMLNS_SAML_PROTOCOL}\" \
         xmlns:saml=\"{XMLNS_SAML_ASSERTION}\" \
         ID=\"{id}\" Version=\"2.0\" IssueInstant=\"{issue_instant}\" \
         Destination=\"{dest}\" AssertionConsumerServiceURL=\"{acs}\" \
         ProtocolBinding=\"{SAML_BINDING_HTTP_POST}\">\
           <saml:Issuer>{issuer}</saml:Issuer>\
           <samlp:NameIDPolicy Format=\"urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress\" AllowCreate=\"true\"/>\
         </samlp:AuthnRequest>"
    )
}

fn deflate_and_base64(xml: &str) -> Result<String, std::io::Error> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(xml.as_bytes())?;
    let deflated = encoder.finish()?;
    Ok(STANDARD.encode(deflated))
}

fn parse_saml_datetime(raw: &str) -> Result<DateTime<Utc>, SamlError> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| SamlError::ExpiredAssertion(format!("invalid SAML timestamp '{raw}': {e}")))
}

fn first_attr_value(
    attributes: &HashMap<String, Vec<String>>,
    candidates: &[String],
) -> Option<String> {
    for key in candidates {
        if let Some(vals) = attributes.get(key) {
            if let Some(first) = vals.iter().find(|v| !v.is_empty()) {
                return Some(first.clone());
            }
        }
    }
    None
}

async fn generate_unique_saml_username(
    db: &Database,
    preferred: &str,
    fallback: &str,
) -> Result<String, SamlError> {
    let candidate_src = if preferred.trim().is_empty() {
        fallback
    } else {
        preferred
    };
    let mut base: String = candidate_src
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '@') {
                c
            } else {
                '_'
            }
        })
        .take(140)
        .collect();
    if base.is_empty() {
        base = "saml_user".to_string();
    }

    if User::objects()
        .filter(q!(username = base.as_str()))?
        .first(db)
        .await?
        .is_none()
    {
        return Ok(base);
    }

    for suffix in 1_u32..=100 {
        let candidate = format!("{base}_{suffix}");
        if User::objects()
            .filter(q!(username = candidate.as_str()))?
            .first(db)
            .await?
            .is_none()
        {
            return Ok(candidate);
        }
    }

    Ok(format!("saml_{}", random_hex_id(8)))
}

async fn load_active_idp(db: &Database, slug: &str) -> Result<SamlIdentityProvider, SamlError> {
    let idp = SamlIdentityProvider::objects()
        .filter(q!(slug = slug))?
        .first(db)
        .await?
        .ok_or_else(|| SamlError::UnknownProvider(slug.to_string()))?;
    if !idp.is_active {
        return Err(SamlError::InactiveProvider(slug.to_string()));
    }
    Ok(idp)
}

fn require_idp_slug(params: &PathParams) -> Result<String, DjangorsError> {
    params
        .get("idp")
        .map(ToString::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DjangorsError::BadRequest("missing SAML idp path parameter".to_string()))
}

fn require_db(req: &Request) -> Result<&Database, DjangorsError> {
    req.state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))
}

fn require_sp_config(req: &Request) -> Result<SamlSpConfig, DjangorsError> {
    req.state::<SamlSpConfig>()
        .cloned()
        .ok_or_else(|| DjangorsError::Internal("SamlSpConfig state absent".to_string()))
}

fn require_cache(req: &Request) -> Result<Arc<dyn Cache>, DjangorsError> {
    req.state::<Arc<dyn Cache>>()
        .cloned()
        .ok_or_else(|| DjangorsError::Internal("Cache state absent".to_string()))
}

fn require_session(req: &Request) -> Result<&Session, DjangorsError> {
    req.ext::<Session>()
        .ok_or_else(|| DjangorsError::Internal("session middleware absent".to_string()))
}

fn sanitize_next_url(next: Option<&str>, default: &str) -> String {
    match next.map(str::trim) {
        Some(n) if n.starts_with('/') && !n.starts_with("//") => n.to_string(),
        _ => default.to_string(),
    }
}

fn extract_query_param(query: Option<&str>, key: &str) -> Option<String> {
    let q_str = query?;
    url::form_urlencoded::parse(q_str.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn random_hex_id(bytes_len: usize) -> String {
    let mut bytes = vec![0_u8; bytes_len];
    OsRng.fill_bytes(&mut bytes);
    let mut out = String::with_capacity(bytes_len * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn strip_pem_headers(pem: &str) -> String {
    pem.lines()
        .filter(|line| !line.starts_with("-----BEGIN ") && !line.starts_with("-----END "))
        .collect::<Vec<_>>()
        .join("")
}

fn escape_xml(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
mod fixture_tests;
