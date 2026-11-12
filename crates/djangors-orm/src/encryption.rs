//! Transparent encryption for `#[djangors(encrypted)]` text fields.
//!
//! An encrypted value is stored as `enc:v1:` followed by the standard base64 of
//! `nonce || ciphertext`, where the nonce is 12 fresh random bytes and the
//! ciphertext is AES-256-GCM output including its 16-byte authentication tag.
//!
//! Keys come from the environment, read once on first use:
//!
//! - `DJANGORS_FIELD_ENCRYPTION_KEY`: standard base64 of exactly 32 bytes. Used
//!   for all new writes.
//! - `DJANGORS_FIELD_ENCRYPTION_OLD_KEYS` (optional): comma-separated base64 keys
//!   that are only tried when decrypting, so values written under a previous key
//!   stay readable during rotation.

use crate::error::OrmError;
use crate::expr::Value;
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::Aes256Gcm;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::sync::OnceLock;

const PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const KEY_ENV: &str = "DJANGORS_FIELD_ENCRYPTION_KEY";
const OLD_KEYS_ENV: &str = "DJANGORS_FIELD_ENCRYPTION_OLD_KEYS";

struct KeyRing {
    current: [u8; KEY_LEN],
    old: Vec<[u8; KEY_LEN]>,
}

/// The loaded keys, or the message explaining why they could not be loaded.
static KEY_RING: OnceLock<Result<KeyRing, String>> = OnceLock::new();

fn key_ring() -> Result<&'static KeyRing, OrmError> {
    let loaded = KEY_RING.get_or_init(load_from_env);
    match loaded {
        Ok(ring) => Ok(ring),
        Err(message) => Err(OrmError::InvalidQuery(message.clone())),
    }
}

fn load_from_env() -> Result<KeyRing, String> {
    let current_raw = std::env::var(KEY_ENV).map_err(|_| {
        format!("field encryption key is not set: {KEY_ENV} must hold a base64-encoded 32-byte key")
    })?;
    let current = parse_key(current_raw.trim())?;
    let old = match std::env::var(OLD_KEYS_ENV) {
        Ok(raw) => raw
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(parse_key)
            .collect::<Result<Vec<_>, _>>()?,
        Err(_) => Vec::new(),
    };
    Ok(KeyRing { current, old })
}

fn parse_key(encoded: &str) -> Result<[u8; KEY_LEN], String> {
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| "field encryption key is not valid base64".to_string())?;
    <[u8; KEY_LEN]>::try_from(bytes.as_slice())
        .map_err(|_| format!("field encryption key must decode to {KEY_LEN} bytes"))
}

/// Installs the encryption keys directly, bypassing the environment.
///
/// Intended for tests only. The first successful call (or the first use of the
/// environment keys) wins; later calls are ignored.
#[doc(hidden)]
pub fn set_keys_for_tests(current: [u8; 32], old: Vec<[u8; 32]>) {
    let _ = KEY_RING.set(Ok(KeyRing { current, old }));
}

// aes-gcm 0.10 takes generic-array 0.14 types, which that crate now marks deprecated.
#[allow(deprecated)]
fn seal(key: &[u8; KEY_LEN], plain: &[u8]) -> Result<Vec<u8>, OrmError> {
    let cipher = Aes256Gcm::new(aes_gcm::aead::generic_array::GenericArray::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plain)
        .map_err(|_| OrmError::InvalidQuery("field encryption failed".to_string()))?;
    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(nonce.as_slice());
    blob.extend_from_slice(&ciphertext);
    Ok(blob)
}

// aes-gcm 0.10 takes generic-array 0.14 types, which that crate now marks deprecated.
#[allow(deprecated)]
fn open(key: &[u8; KEY_LEN], blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() < NONCE_LEN {
        return None;
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(aes_gcm::aead::generic_array::GenericArray::from_slice(key));
    let nonce = aes_gcm::aead::generic_array::GenericArray::from_slice(nonce_bytes);
    cipher.decrypt(nonce, ciphertext).ok()
}

/// Encrypts `plain` with the current key and returns the `enc:v1:` form.
///
/// Every call uses a fresh random nonce, so encrypting the same text twice gives
/// different output.
pub fn encrypt_str(plain: &str) -> Result<String, OrmError> {
    let ring = key_ring()?;
    let blob = seal(&ring.current, plain.as_bytes())?;
    Ok(format!("{PREFIX}{}", STANDARD.encode(blob)))
}

/// Decrypts a stored field value.
///
/// Values without the `enc:v1:` prefix are legacy plaintext and are returned
/// unchanged, so a column can be migrated to encryption in place. Encrypted
/// values are tried with the current key and then each old key. If none of them
/// authenticates, an error is returned; ciphertext is never returned as text.
pub fn decrypt_str(stored: &str) -> Result<String, OrmError> {
    let encoded = match stored.strip_prefix(PREFIX) {
        Some(encoded) => encoded,
        None => return Ok(stored.to_string()),
    };
    let blob = STANDARD.decode(encoded).map_err(|_| {
        OrmError::InvalidQuery("encrypted field value is not valid base64".to_string())
    })?;
    let ring = key_ring()?;
    let plain = std::iter::once(&ring.current)
        .chain(ring.old.iter())
        .find_map(|key| open(key, &blob))
        .ok_or_else(|| {
            OrmError::InvalidQuery(
                "encrypted field could not be decrypted with any configured key".to_string(),
            )
        })?;
    String::from_utf8(plain)
        .map_err(|_| OrmError::InvalidQuery("decrypted field value is not valid UTF-8".to_string()))
}

/// Encrypts a bind value for an encrypted field. `NULL` passes through; text is
/// encrypted; any other kind of value is rejected so plaintext is never written.
///
/// Used by the generated `save`/`update` code and by the bulk write paths.
pub fn encrypt_value(value: Value) -> Result<Value, OrmError> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Text(text) => Ok(Value::Text(encrypt_str(&text)?)),
        _ => Err(OrmError::InvalidQuery(
            "encrypted fields accept only text values".to_string(),
        )),
    }
}

#[cfg(test)]
#[path = "encryption_tests.rs"]
mod tests;
