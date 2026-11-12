//! Passkeys (WebAuthn) registration and sign-in, with no OpenSSL dependency.
//!
//! Available with the `passkeys` feature. Djangors acts as the WebAuthn *relying party*:
//!
//! 1. [`start_registration`] returns options for the browser's
//!    `navigator.credentials.create()` plus a [`RegistrationState`] to keep in the
//!    session; [`finish_registration`] verifies the browser's answer and returns a
//!    [`Passkey`] to store.
//! 2. [`start_authentication`] returns options for `navigator.credentials.get()` plus an
//!    [`AuthenticationState`]; [`finish_authentication`] verifies the signature against
//!    the stored [`Passkey`] and returns its new signature counter.
//!
//! Supported signature algorithms: ES256 (P-256, COSE -7) and EdDSA (Ed25519, COSE -8).
//! These cover platform authenticators (Apple, Google, Windows Hello, Android) and
//! security keys. Attestation is requested as `"none"`: Djangors does not decide which
//! authenticator makes are allowed, so attestation statements are not verified.
//! User verification (PIN, fingerprint, face) is required for every ceremony.
//!
//! Browser side, use `PublicKeyCredential.parseCreationOptionsFromJSON(options.publicKey)`
//! / `parseRequestOptionsFromJSON(...)` on the options, and send `credential.toJSON()` back;
//! the request structs here match that JSON.

mod cbor;
#[cfg(test)]
mod tests;

pub mod store;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use store::PasskeyStore;

const FLAG_USER_PRESENT: u8 = 0x01;
const FLAG_USER_VERIFIED: u8 = 0x04;
const FLAG_BACKUP_ELIGIBLE: u8 = 0x08;
const FLAG_BACKED_UP: u8 = 0x10;
const FLAG_ATTESTED_DATA: u8 = 0x40;
const FLAG_EXTENSIONS: u8 = 0x80;

const COSE_ALG_ES256: i128 = -7;
const COSE_ALG_EDDSA: i128 = -8;
const MAX_CREDENTIAL_ID_LEN: usize = 1023;
const TIMEOUT_MS: u64 = 300_000;

/// Why a passkey ceremony was rejected.
#[derive(Debug, thiserror::Error)]
pub enum PasskeyError {
    /// The browser's response was malformed or missing a field.
    #[error("invalid passkey response: {0}")]
    InvalidResponse(String),
    /// The response answers a different challenge than the one issued.
    #[error("passkey challenge does not match")]
    ChallengeMismatch,
    /// The response came from a page whose origin is not allowed.
    #[error("passkey origin `{0}` is not allowed")]
    OriginMismatch(String),
    /// The authenticator signed for a different relying party ID.
    #[error("passkey relying party ID does not match")]
    RpIdMismatch,
    /// The authenticator did not report a user gesture.
    #[error("user presence was not confirmed")]
    UserNotPresent,
    /// The authenticator did not verify the user (PIN or biometric).
    #[error("user verification was not performed")]
    UserNotVerified,
    /// The credential uses a signature algorithm Djangors does not support.
    #[error("unsupported passkey algorithm {0}")]
    UnsupportedAlgorithm(i64),
    /// The signature did not verify.
    #[error("passkey signature is invalid")]
    BadSignature,
    /// The signature counter went backwards, which suggests a cloned authenticator.
    #[error("passkey signature counter did not increase")]
    CounterRegression,
    /// The response is for a different credential or user than expected.
    #[error("passkey credential does not match")]
    CredentialMismatch,
    /// Storing or loading a passkey failed.
    #[error("passkey storage error: {0}")]
    Database(String),
}

impl From<PasskeyError> for djangors_core::DjangorsError {
    fn from(err: PasskeyError) -> Self {
        match err {
            PasskeyError::Database(msg) => {
                djangors_core::DjangorsError::Internal(format!("passkey storage error: {msg}"))
            }
            other => djangors_core::DjangorsError::api(
                djangors_core::StatusCode::BAD_REQUEST,
                "passkey_invalid",
                other.to_string(),
            ),
        }
    }
}

/// The relying party: your site, as passkeys see it.
#[derive(Debug, Clone)]
pub struct RelyingParty {
    id: String,
    name: String,
    origins: Vec<String>,
}

impl RelyingParty {
    /// `id` is the registrable domain passkeys are bound to (e.g. `"example.com"`);
    /// `name` is shown by the browser; `origin` is the exact page origin
    /// (e.g. `"https://example.com"`).
    pub fn new(id: impl Into<String>, name: impl Into<String>, origin: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            origins: vec![origin.into()],
        }
    }

    /// Allows another exact origin, such as a subdomain (`"https://app.example.com"`).
    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origins.push(origin.into());
        self
    }

    /// The relying party ID.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A registered passkey. Store it; [`PasskeyStore`] does this in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passkey {
    /// The credential ID chosen by the authenticator.
    pub credential_id: Vec<u8>,
    /// The credential public key, as COSE_Key bytes.
    pub public_key: Vec<u8>,
    /// The last signature counter seen (0 for authenticators without a counter).
    pub sign_count: u32,
    /// The user handle the passkey was registered for.
    pub user_handle: Vec<u8>,
    /// Whether the passkey may be synced to other devices.
    pub backup_eligible: bool,
}

/// Server-side state between [`start_registration`] and [`finish_registration`].
/// Keep it in the user's session; it must not be reused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationState {
    challenge: String,
    user_handle: Vec<u8>,
}

/// Server-side state between [`start_authentication`] and [`finish_authentication`].
/// Keep it in the session; it must not be reused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationState {
    challenge: String,
}

/// The JSON a browser produces with `credential.toJSON()` after `credentials.create()`.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationResponse {
    /// Base64url credential ID.
    #[serde(rename = "rawId")]
    pub raw_id: String,
    /// Always `"public-key"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The authenticator's response.
    pub response: AttestationResponse,
}

/// The `response` member of a [`RegistrationResponse`].
#[derive(Debug, Clone, Deserialize)]
pub struct AttestationResponse {
    /// Base64url client data JSON.
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    /// Base64url CBOR attestation object.
    #[serde(rename = "attestationObject")]
    pub attestation_object: String,
}

/// The JSON a browser produces with `credential.toJSON()` after `credentials.get()`.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthenticationResponse {
    /// Base64url credential ID; use it to look up the stored [`Passkey`].
    #[serde(rename = "rawId")]
    pub raw_id: String,
    /// Always `"public-key"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The authenticator's response.
    pub response: AssertionResponse,
}

/// The `response` member of an [`AuthenticationResponse`].
#[derive(Debug, Clone, Deserialize)]
pub struct AssertionResponse {
    /// Base64url client data JSON.
    #[serde(rename = "clientDataJSON")]
    pub client_data_json: String,
    /// Base64url authenticator data.
    #[serde(rename = "authenticatorData")]
    pub authenticator_data: String,
    /// Base64url signature.
    pub signature: String,
    /// Base64url user handle (present for discoverable credentials).
    #[serde(rename = "userHandle", default)]
    pub user_handle: Option<String>,
}

impl AuthenticationResponse {
    /// The decoded credential ID, for looking up the stored passkey.
    pub fn credential_id(&self) -> Result<Vec<u8>, PasskeyError> {
        b64_decode(&self.raw_id, "rawId")
    }
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: Option<bool>,
}

/// Returns an opaque user handle for a numeric user ID (8 bytes, big-endian).
pub fn user_handle_for(user_id: i64) -> Vec<u8> {
    user_id.to_be_bytes().to_vec()
}

/// Begins registering a passkey for a user. `exclude` lists the user's existing passkeys
/// so the same authenticator isn't registered twice.
pub fn start_registration(
    rp: &RelyingParty,
    user_handle: &[u8],
    user_name: &str,
    display_name: &str,
    exclude: &[Passkey],
) -> (serde_json::Value, RegistrationState) {
    let challenge = new_challenge();
    let exclude: Vec<_> = exclude
        .iter()
        .map(|p| serde_json::json!({ "type": "public-key", "id": URL_SAFE_NO_PAD.encode(&p.credential_id) }))
        .collect();
    let options = serde_json::json!({
        "publicKey": {
            "rp": { "id": rp.id, "name": rp.name },
            "user": {
                "id": URL_SAFE_NO_PAD.encode(user_handle),
                "name": user_name,
                "displayName": display_name,
            },
            "challenge": challenge,
            "pubKeyCredParams": [
                { "type": "public-key", "alg": COSE_ALG_ES256 as i64 },
                { "type": "public-key", "alg": COSE_ALG_EDDSA as i64 },
            ],
            "timeout": TIMEOUT_MS,
            "attestation": "none",
            "authenticatorSelection": {
                "residentKey": "required",
                "requireResidentKey": true,
                "userVerification": "required",
            },
            "excludeCredentials": exclude,
        }
    });
    let state = RegistrationState {
        challenge,
        user_handle: user_handle.to_vec(),
    };
    (options, state)
}

/// Verifies a registration response and returns the new [`Passkey`] to store.
pub fn finish_registration(
    rp: &RelyingParty,
    state: &RegistrationState,
    response: &RegistrationResponse,
) -> Result<Passkey, PasskeyError> {
    if response.kind != "public-key" {
        return Err(invalid("type must be public-key"));
    }
    let client_data_json = b64_decode(&response.response.client_data_json, "clientDataJSON")?;
    verify_client_data(rp, &client_data_json, "webauthn.create", &state.challenge)?;

    let attestation = b64_decode(&response.response.attestation_object, "attestationObject")?;
    let attestation = cbor::decode(&attestation)?;
    let auth_data = attestation
        .get_text("authData")
        .and_then(cbor::Value::as_bytes)
        .ok_or_else(|| invalid("attestation object has no authData"))?;
    if attestation
        .get_text("fmt")
        .and_then(cbor::Value::as_text)
        .is_none()
    {
        return Err(invalid("attestation object has no fmt"));
    }

    let parsed = parse_auth_data(auth_data)?;
    check_rp_and_flags(rp, &parsed)?;
    let attested = parsed
        .attested
        .ok_or_else(|| invalid("registration has no attested credential data"))?;

    let raw_id = b64_decode(&response.raw_id, "rawId")?;
    if raw_id != attested.credential_id {
        return Err(PasskeyError::CredentialMismatch);
    }
    // Rejects unsupported algorithms and invalid keys at registration time.
    PublicKey::from_cose(&attested.public_key)?;

    Ok(Passkey {
        credential_id: attested.credential_id,
        public_key: attested.public_key,
        sign_count: parsed.sign_count,
        user_handle: state.user_handle.clone(),
        backup_eligible: parsed.flags & FLAG_BACKUP_ELIGIBLE != 0,
    })
}

/// Begins a sign-in. Pass the user's passkeys to restrict the choice, or an empty slice
/// to let the browser offer any passkey for this site (username-less sign-in).
pub fn start_authentication(
    rp: &RelyingParty,
    allow: &[Passkey],
) -> (serde_json::Value, AuthenticationState) {
    let challenge = new_challenge();
    let allow: Vec<_> = allow
        .iter()
        .map(|p| serde_json::json!({ "type": "public-key", "id": URL_SAFE_NO_PAD.encode(&p.credential_id) }))
        .collect();
    let options = serde_json::json!({
        "publicKey": {
            "challenge": challenge,
            "rpId": rp.id,
            "timeout": TIMEOUT_MS,
            "userVerification": "required",
            "allowCredentials": allow,
        }
    });
    (options, AuthenticationState { challenge })
}

/// Verifies a sign-in response against the stored passkey (look it up with
/// [`AuthenticationResponse::credential_id`]). Returns the new signature counter, which
/// you must save (see [`PasskeyStore::update_sign_count`]).
pub fn finish_authentication(
    rp: &RelyingParty,
    state: &AuthenticationState,
    response: &AuthenticationResponse,
    passkey: &Passkey,
) -> Result<u32, PasskeyError> {
    if response.kind != "public-key" {
        return Err(invalid("type must be public-key"));
    }
    if response.credential_id()? != passkey.credential_id {
        return Err(PasskeyError::CredentialMismatch);
    }
    if let Some(handle) = &response.response.user_handle {
        if !handle.is_empty() && b64_decode(handle, "userHandle")? != passkey.user_handle {
            return Err(PasskeyError::CredentialMismatch);
        }
    }

    let client_data_json = b64_decode(&response.response.client_data_json, "clientDataJSON")?;
    verify_client_data(rp, &client_data_json, "webauthn.get", &state.challenge)?;

    let auth_data = b64_decode(&response.response.authenticator_data, "authenticatorData")?;
    let parsed = parse_auth_data(&auth_data)?;
    check_rp_and_flags(rp, &parsed)?;
    if parsed.attested.is_some() {
        return Err(invalid("sign-in must not contain attested credential data"));
    }

    let signature = b64_decode(&response.response.signature, "signature")?;
    let mut signed = auth_data.clone();
    signed.extend_from_slice(&Sha256::digest(&client_data_json));
    PublicKey::from_cose(&passkey.public_key)?.verify(&signed, &signature)?;

    // A counter that stops increasing means two copies of the key exist.
    if (parsed.sign_count != 0 || passkey.sign_count != 0)
        && parsed.sign_count <= passkey.sign_count
    {
        return Err(PasskeyError::CounterRegression);
    }
    Ok(parsed.sign_count)
}

fn new_challenge() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn invalid(reason: &str) -> PasskeyError {
    PasskeyError::InvalidResponse(reason.to_string())
}

fn b64_decode(value: &str, field: &str) -> Result<Vec<u8>, PasskeyError> {
    URL_SAFE_NO_PAD
        .decode(value.trim_end_matches('='))
        .map_err(|_| invalid(&format!("{field} is not base64url")))
}

fn verify_client_data(
    rp: &RelyingParty,
    client_data_json: &[u8],
    expected_type: &str,
    expected_challenge: &str,
) -> Result<(), PasskeyError> {
    let client: ClientData = serde_json::from_slice(client_data_json)
        .map_err(|_| invalid("clientDataJSON is not valid JSON"))?;
    if client.kind != expected_type {
        return Err(invalid(&format!(
            "clientDataJSON type must be {expected_type}"
        )));
    }
    if client.challenge.trim_end_matches('=') != expected_challenge {
        return Err(PasskeyError::ChallengeMismatch);
    }
    if !rp.origins.contains(&client.origin) {
        return Err(PasskeyError::OriginMismatch(client.origin));
    }
    if client.cross_origin == Some(true) {
        return Err(invalid("cross-origin requests are not allowed"));
    }
    Ok(())
}

struct AttestedCredential {
    credential_id: Vec<u8>,
    public_key: Vec<u8>,
}

struct AuthData {
    rp_id_hash: [u8; 32],
    flags: u8,
    sign_count: u32,
    attested: Option<AttestedCredential>,
}

/// Parses authenticator data (WebAuthn §6.1), requiring it to be exactly consumed.
fn parse_auth_data(data: &[u8]) -> Result<AuthData, PasskeyError> {
    if data.len() < 37 {
        return Err(invalid("authenticator data is too short"));
    }
    let mut rp_id_hash = [0u8; 32];
    rp_id_hash.copy_from_slice(&data[..32]);
    let flags = data[32];
    let sign_count = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);
    let mut pos = 37;

    let attested = if flags & FLAG_ATTESTED_DATA != 0 {
        // aaguid (16) + credential ID length (2)
        let header = data
            .get(pos..pos + 18)
            .ok_or_else(|| invalid("attested credential data is truncated"))?;
        let id_len = usize::from(u16::from_be_bytes([header[16], header[17]]));
        if id_len == 0 || id_len > MAX_CREDENTIAL_ID_LEN {
            return Err(invalid("credential ID length is out of range"));
        }
        pos += 18;
        let credential_id = data
            .get(pos..pos + id_len)
            .ok_or_else(|| invalid("credential ID is truncated"))?
            .to_vec();
        pos += id_len;
        let (_, key_len) = cbor::decode_prefix(&data[pos..])?;
        let public_key = data[pos..pos + key_len].to_vec();
        pos += key_len;
        Some(AttestedCredential {
            credential_id,
            public_key,
        })
    } else {
        None
    };

    if flags & FLAG_EXTENSIONS != 0 {
        let (extensions, used) = cbor::decode_prefix(&data[pos..])?;
        if !matches!(extensions, cbor::Value::Map(_)) {
            return Err(invalid("authenticator extensions must be a map"));
        }
        pos += used;
    }
    if pos != data.len() {
        return Err(invalid("authenticator data has trailing bytes"));
    }
    Ok(AuthData {
        rp_id_hash,
        flags,
        sign_count,
        attested,
    })
}

fn check_rp_and_flags(rp: &RelyingParty, auth: &AuthData) -> Result<(), PasskeyError> {
    let expected: [u8; 32] = Sha256::digest(rp.id.as_bytes()).into();
    if auth.rp_id_hash != expected {
        return Err(PasskeyError::RpIdMismatch);
    }
    if auth.flags & FLAG_USER_PRESENT == 0 {
        return Err(PasskeyError::UserNotPresent);
    }
    if auth.flags & FLAG_USER_VERIFIED == 0 {
        return Err(PasskeyError::UserNotVerified);
    }
    if auth.flags & FLAG_BACKED_UP != 0 && auth.flags & FLAG_BACKUP_ELIGIBLE == 0 {
        return Err(invalid(
            "backed-up flag set on a credential that is not backup eligible",
        ));
    }
    Ok(())
}

enum PublicKey {
    Es256(p256::ecdsa::VerifyingKey),
    Ed25519(ed25519_dalek::VerifyingKey),
}

impl PublicKey {
    /// Parses a COSE_Key (RFC 9053), accepting only ES256 on P-256 and EdDSA on Ed25519.
    fn from_cose(bytes: &[u8]) -> Result<Self, PasskeyError> {
        let key = cbor::decode(bytes)?;
        let int = |label: i128| key.get_int(label).and_then(cbor::Value::as_int);
        let bytes32 = |label: i128| -> Result<&[u8], PasskeyError> {
            key.get_int(label)
                .and_then(cbor::Value::as_bytes)
                .filter(|b| b.len() == 32)
                .ok_or_else(|| invalid("COSE key coordinate is missing or not 32 bytes"))
        };
        let alg = int(3).ok_or_else(|| invalid("COSE key has no alg"))?;
        let (kty, crv) = (int(1), int(-1));
        match alg {
            COSE_ALG_ES256 if kty == Some(2) && crv == Some(1) => {
                let mut sec1 = Vec::with_capacity(65);
                sec1.push(0x04);
                sec1.extend_from_slice(bytes32(-2)?);
                sec1.extend_from_slice(bytes32(-3)?);
                p256::ecdsa::VerifyingKey::from_sec1_bytes(&sec1)
                    .map(PublicKey::Es256)
                    .map_err(|_| invalid("P-256 public key is not a valid curve point"))
            }
            COSE_ALG_EDDSA if kty == Some(1) && crv == Some(6) => {
                let mut x = [0u8; 32];
                x.copy_from_slice(bytes32(-2)?);
                ed25519_dalek::VerifyingKey::from_bytes(&x)
                    .map(PublicKey::Ed25519)
                    .map_err(|_| invalid("Ed25519 public key is invalid"))
            }
            COSE_ALG_ES256 | COSE_ALG_EDDSA => {
                Err(invalid("COSE key type or curve does not match alg"))
            }
            other => Err(PasskeyError::UnsupportedAlgorithm(
                i64::try_from(other).unwrap_or(i64::MIN),
            )),
        }
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), PasskeyError> {
        match self {
            PublicKey::Es256(key) => {
                use p256::ecdsa::signature::Verifier;
                // WebAuthn ES256 signatures are ASN.1 DER encoded.
                let sig = p256::ecdsa::Signature::from_der(signature)
                    .map_err(|_| PasskeyError::BadSignature)?;
                let sig = sig.normalize_s().unwrap_or(sig);
                key.verify(message, &sig)
                    .map_err(|_| PasskeyError::BadSignature)
            }
            PublicKey::Ed25519(key) => {
                let sig = ed25519_dalek::Signature::from_slice(signature)
                    .map_err(|_| PasskeyError::BadSignature)?;
                key.verify_strict(message, &sig)
                    .map_err(|_| PasskeyError::BadSignature)
            }
        }
    }
}
