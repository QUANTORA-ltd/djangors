//! Signing API (Django's `django.core.signing`) and signed URLs.
//!
//! Every signature is an HMAC-SHA256 over the value, keyed with a key derived from the
//! secret and a caller-supplied *salt*. The salt is domain separation: a token signed for
//! one purpose (say `"myapp.email_confirm"`) does not verify under another salt, even when
//! both use the same `SECRET_KEY`. Signatures are compared in constant time.
//!
//! Key rotation: [`Signer::from_settings`] signs with `SECRET_KEY` and accepts signatures
//! made with any key in `SECRET_KEY_FALLBACKS`. A missing or empty key is an error, so
//! nothing is ever signed with an empty secret.
//!
//! ```ignore
//! use djangors_core::signing::{dumps, loads, Signer, TimestampSigner};
//! use std::time::Duration;
//!
//! let signer = Signer::new("a-long-secret", "myapp.invite")?;
//! let token = dumps(&serde_json::json!({"org": 7}), &signer, true)?;
//! let back: serde_json::Value = loads(&token, &signer)?;
//!
//! let ts = TimestampSigner::new("a-long-secret", "myapp.reset", Duration::from_secs(3600))?;
//! let token = ts.sign_value("user-42");
//! let user = ts.unsign_value(&token)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::error::DjangorsError;
use crate::extract::FromRequest;
use crate::request::Request;
use crate::settings::DjangorsSettings;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{de::DeserializeOwned, Serialize};
use sha2::Sha256;
use std::io::{Read, Write};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Salt used by [`SignedUrl`] and [`sign_url`] unless the caller supplies another one.
pub const SIGNED_URL_SALT: &str = "djangors.signed_url.v1";

/// Largest payload accepted after decompression by [`loads`] (guards against zip bombs).
const MAX_DECOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;

/// Allowed forward clock skew for timestamps produced by another server.
const MAX_CLOCK_SKEW_SECS: u64 = 60;

/// Errors returned by the signing API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SigningError {
    /// The secret key (or a fallback key) is empty or absent. Signing fails closed.
    #[error("signing key is missing or empty")]
    MissingKey,
    /// The signature does not match any configured key, or the salt differs.
    #[error("bad signature")]
    BadSignature,
    /// The signed timestamp is older than the allowed maximum age.
    #[error("signature has expired")]
    Expired,
    /// The token does not have the expected structure.
    #[error("malformed signed value")]
    Malformed,
    /// The value could not be serialized to JSON.
    #[error("serialization failed: {0}")]
    Serialize(String),
    /// The system clock is unavailable.
    #[error("system clock is unavailable")]
    Clock,
}

/// Derives the per-salt HMAC key from one raw secret.
fn derive_key(secret: &[u8], salt: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(b"djangors.signing.v1|");
    mac.update(salt.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

fn now_secs() -> Result<u64, SigningError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| SigningError::Clock)
}

/// Signs and verifies string values with HMAC-SHA256 under a salt.
///
/// Output has the form `value:signature`, where the signature is unpadded base64url.
#[derive(Clone)]
pub struct Signer {
    salt: String,
    /// Derived keys, newest first. Index 0 signs; every key verifies.
    keys: Vec<Vec<u8>>,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer")
            .field("salt", &self.salt)
            .field("keys", &format_args!("[{} redacted]", self.keys.len()))
            .finish()
    }
}

impl Signer {
    /// Creates a signer for `key` under `salt`. Fails with [`SigningError::MissingKey`]
    /// when `key` is empty.
    pub fn new(key: impl AsRef<[u8]>, salt: &str) -> Result<Self, SigningError> {
        let key = key.as_ref();
        if key.is_empty() {
            return Err(SigningError::MissingKey);
        }
        Ok(Self {
            salt: salt.to_owned(),
            keys: vec![derive_key(key, salt)],
        })
    }

    /// Adds keys that are accepted for verification only. Fails if any is empty.
    pub fn with_fallbacks<I, K>(mut self, fallbacks: I) -> Result<Self, SigningError>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<[u8]>,
    {
        for key in fallbacks {
            let key = key.as_ref();
            if key.is_empty() {
                return Err(SigningError::MissingKey);
            }
            self.keys.push(derive_key(key, &self.salt));
        }
        Ok(self)
    }

    /// Builds a signer from `SECRET_KEY` and `SECRET_KEY_FALLBACKS`.
    pub fn from_settings(settings: &DjangorsSettings, salt: &str) -> Result<Self, SigningError> {
        Self::new(&settings.secret_key, salt)?.with_fallbacks(&settings.secret_key_fallbacks)
    }

    /// Returns the salt this signer was created with.
    pub fn salt(&self) -> &str {
        &self.salt
    }

    /// Returns `value:signature`.
    pub fn sign(&self, value: &str) -> String {
        let sig = hmac_sha256(&self.keys[0], value.as_bytes());
        format!("{value}:{}", B64.encode(sig))
    }

    /// Verifies a token produced by [`Signer::sign`] and returns the original value.
    pub fn unsign(&self, signed: &str) -> Result<String, SigningError> {
        let (value, sig_b64) = signed.rsplit_once(':').ok_or(SigningError::Malformed)?;
        let sig = B64
            .decode(sig_b64)
            .map_err(|_| SigningError::BadSignature)?;
        let valid = self.keys.iter().any(|key| {
            let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
            mac.update(value.as_bytes());
            mac.verify_slice(&sig).is_ok()
        });
        if valid {
            Ok(value.to_owned())
        } else {
            Err(SigningError::BadSignature)
        }
    }
}

/// Signs and verifies values; implemented by [`Signer`] and [`TimestampSigner`].
pub trait ValueSigner {
    /// Signs `value` and returns the token.
    fn sign_value(&self, value: &str) -> String;
    /// Verifies `signed` and returns the original value.
    fn unsign_value(&self, signed: &str) -> Result<String, SigningError>;
}

impl ValueSigner for Signer {
    fn sign_value(&self, value: &str) -> String {
        self.sign(value)
    }

    fn unsign_value(&self, signed: &str) -> Result<String, SigningError> {
        self.unsign(signed)
    }
}

/// A [`Signer`] that also embeds a Unix timestamp and rejects values older than `max_age`.
///
/// Output has the form `value:timestamp:signature`. The timestamp is covered by the signature.
#[derive(Clone, Debug)]
pub struct TimestampSigner {
    inner: Signer,
    max_age: Duration,
}

impl TimestampSigner {
    /// Creates a timestamp signer. Fails if `key` is empty.
    pub fn new(key: impl AsRef<[u8]>, salt: &str, max_age: Duration) -> Result<Self, SigningError> {
        Ok(Self {
            inner: Signer::new(key, salt)?,
            max_age,
        })
    }

    /// Builds a timestamp signer from the settings' `SECRET_KEY` and fallbacks.
    pub fn from_settings(
        settings: &DjangorsSettings,
        salt: &str,
        max_age: Duration,
    ) -> Result<Self, SigningError> {
        Ok(Self {
            inner: Signer::from_settings(settings, salt)?,
            max_age,
        })
    }

    /// Adds keys that are accepted for verification only. Fails if any is empty.
    pub fn with_fallbacks<I, K>(mut self, fallbacks: I) -> Result<Self, SigningError>
    where
        I: IntoIterator<Item = K>,
        K: AsRef<[u8]>,
    {
        self.inner = self.inner.with_fallbacks(fallbacks)?;
        Ok(self)
    }

    /// Signs `value` with the current timestamp (`value:timestamp:signature`).
    pub fn sign(&self, value: &str) -> String {
        self.sign_value(value)
    }

    /// Verifies `signed` and rejects timestamps older than `max_age`.
    pub fn unsign(&self, signed: &str) -> Result<String, SigningError> {
        self.unsign_value(signed)
    }

    /// Signs `value` with the timestamp `ts` (Unix seconds).
    pub(crate) fn sign_at(&self, value: &str, ts: u64) -> String {
        self.inner.sign(&format!("{value}:{ts}"))
    }

    /// Verifies `signed` as of `now` (Unix seconds).
    pub(crate) fn unsign_at(&self, signed: &str, now: u64) -> Result<String, SigningError> {
        let inner = self.inner.unsign(signed)?;
        let (value, ts_str) = inner.rsplit_once(':').ok_or(SigningError::Malformed)?;
        let ts: u64 = ts_str.parse().map_err(|_| SigningError::Malformed)?;
        if ts > now.saturating_add(MAX_CLOCK_SKEW_SECS) {
            return Err(SigningError::Malformed);
        }
        if now.saturating_sub(ts) > self.max_age.as_secs() {
            return Err(SigningError::Expired);
        }
        Ok(value.to_owned())
    }
}

impl ValueSigner for TimestampSigner {
    fn sign_value(&self, value: &str) -> String {
        // Clock failure cannot produce a valid timestamp; `now_secs` errors surface on verify.
        let ts = now_secs().unwrap_or(0);
        self.sign_at(value, ts)
    }

    fn unsign_value(&self, signed: &str) -> Result<String, SigningError> {
        let now = now_secs()?;
        self.unsign_at(signed, now)
    }
}

/// Serializes `value` to JSON, optionally compresses it, and signs it with `signer`.
///
/// Compression (zlib) is used only when it makes the payload smaller. Compressed payloads
/// are marked with a leading `.` inside the signed value, as in Django.
pub fn dumps<S: ValueSigner, T: Serialize>(
    value: &T,
    signer: &S,
    compress: bool,
) -> Result<String, SigningError> {
    let json = serde_json::to_vec(value).map_err(|e| SigningError::Serialize(e.to_string()))?;
    let (payload, prefix) = if compress {
        let packed = zlib_compress(&json)?;
        if packed.len() < json.len() {
            (packed, ".")
        } else {
            (json, "")
        }
    } else {
        (json, "")
    };
    Ok(signer.sign_value(&format!("{prefix}{}", B64.encode(payload))))
}

/// Verifies a token made by [`dumps`] and deserializes its payload.
pub fn loads<S: ValueSigner, T: DeserializeOwned>(
    token: &str,
    signer: &S,
) -> Result<T, SigningError> {
    let value = signer.unsign_value(token)?;
    let (compressed, body) = match value.strip_prefix('.') {
        Some(rest) => (true, rest),
        None => (false, value.as_str()),
    };
    let raw = B64.decode(body).map_err(|_| SigningError::Malformed)?;
    let json = if compressed {
        zlib_decompress(&raw)?
    } else {
        raw
    };
    serde_json::from_slice(&json).map_err(|_| SigningError::Malformed)
}

fn zlib_compress(data: &[u8]) -> Result<Vec<u8>, SigningError> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| SigningError::Serialize(e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| SigningError::Serialize(e.to_string()))
}

fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>, SigningError> {
    let decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .take(MAX_DECOMPRESSED_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|_| SigningError::Malformed)?;
    if out.len() as u64 > MAX_DECOMPRESSED_BYTES {
        return Err(SigningError::Malformed);
    }
    Ok(out)
}

/// Query pairs with the reserved `exp` and `sig` values pulled out.
type ReservedSplit<'a> = (Vec<&'a str>, Option<&'a str>, Option<&'a str>);

/// Splits a raw query string into `(pairs, exp, sig)`.
///
/// `exp` and `sig` are removed from the pairs. Duplicates are reported as malformed.
fn split_reserved(query: &str) -> Result<ReservedSplit<'_>, SigningError> {
    let mut pairs = Vec::new();
    let mut exp = None;
    let mut sig = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, val) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "exp" => {
                if exp.replace(val).is_some() {
                    return Err(SigningError::Malformed);
                }
            }
            "sig" => {
                if sig.replace(val).is_some() {
                    return Err(SigningError::Malformed);
                }
            }
            _ => pairs.push(pair),
        }
    }
    Ok((pairs, exp, sig))
}

/// The bytes that are signed for a URL: method, path, sorted query pairs and expiry.
fn canonical_request(method: &str, path: &str, pairs: &[&str], exp: u64) -> String {
    let mut sorted: Vec<&str> = pairs.to_vec();
    sorted.sort_unstable();
    format!(
        "{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path,
        sorted.join("&"),
        exp
    )
}

/// Returns `path_and_query` with `exp` and `sig` parameters appended. The signature covers
/// the HTTP method, the path, the query pairs in sorted order and `expires_at` (Unix seconds).
///
/// Any existing `exp` or `sig` parameters are replaced. Fails if `signer` is unusable.
pub fn sign_url(
    signer: &Signer,
    method: &str,
    path_and_query: &str,
    expires_at: u64,
) -> Result<String, SigningError> {
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let (pairs, _, _) = split_reserved(query)?;
    let canonical = canonical_request(method, path, &pairs, expires_at);
    let sig = B64.encode(hmac_sha256(&signer.keys[0], canonical.as_bytes()));

    let mut out = String::with_capacity(path_and_query.len() + 64);
    out.push_str(path);
    out.push('?');
    for pair in &pairs {
        out.push_str(pair);
        out.push('&');
    }
    out.push_str(&format!("exp={expires_at}&sig={sig}"));
    Ok(out)
}

/// Like [`sign_url`], but expires `ttl` after the current time.
pub fn sign_url_for(
    signer: &Signer,
    method: &str,
    path_and_query: &str,
    ttl: Duration,
) -> Result<String, SigningError> {
    let expires_at = now_secs()?.saturating_add(ttl.as_secs());
    sign_url(signer, method, path_and_query, expires_at)
}

/// Checks a signed URL against `signer` as of `now` (Unix seconds).
///
/// `raw_query` is the undecoded query string, as sent by the client.
pub fn verify_url(
    signer: &Signer,
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    now: u64,
) -> Result<(), SigningError> {
    let (pairs, exp, sig) = split_reserved(raw_query.unwrap_or(""))?;
    let exp: u64 = exp
        .ok_or(SigningError::Malformed)?
        .parse::<u64>()
        .map_err(|_| SigningError::Malformed)?;
    let sig_b64 = sig.ok_or(SigningError::Malformed)?;
    let sig_bytes = B64
        .decode(sig_b64)
        .map_err(|_| SigningError::BadSignature)?;

    let canonical = canonical_request(method, path, &pairs, exp);
    let valid = signer.keys.iter().any(|key| {
        let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
        mac.update(canonical.as_bytes());
        mac.verify_slice(&sig_bytes).is_ok()
    });
    if !valid {
        return Err(SigningError::BadSignature);
    }
    if now >= exp {
        return Err(SigningError::Expired);
    }
    Ok(())
}

/// Extractor for requests whose URL carries a valid `exp` and `sig` (see [`sign_url`]).
///
/// Rejects a bad or tampered link with `403 Forbidden`, and an expired link with `403` too.
/// A missing `SECRET_KEY` fails with an internal error rather than accepting the request.
///
/// ```ignore
/// async fn download(req: Request, _p: PathParams) -> Result<Response, DjangorsError> {
///     let _link = SignedUrl::from_request(&req).await?;
///     Response::text(StatusCode::OK, "file contents")
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedUrl {
    /// The expiry timestamp (Unix seconds) that was verified.
    pub expires_at: u64,
}

#[async_trait::async_trait]
impl FromRequest for SignedUrl {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let settings = req
            .state::<DjangorsSettings>()
            .ok_or_else(|| DjangorsError::Internal("settings state absent".to_string()))?;
        let signer = Signer::from_settings(settings, SIGNED_URL_SALT)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let now = now_secs().map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let expires_at = split_reserved(req.raw_query().unwrap_or(""))
            .ok()
            .and_then(|(_, exp, _)| exp.and_then(|e| e.parse::<u64>().ok()))
            .unwrap_or(0);
        match verify_url(
            &signer,
            req.method().as_str(),
            req.path(),
            req.raw_query(),
            now,
        ) {
            Ok(()) => Ok(SignedUrl { expires_at }),
            Err(SigningError::Expired) => Err(DjangorsError::Forbidden(
                "signed URL has expired".to_string(),
            )),
            Err(_) => Err(DjangorsError::Forbidden("invalid signed URL".to_string())),
        }
    }
}

#[cfg(test)]
#[path = "signing_tests.rs"]
mod signing_tests;
