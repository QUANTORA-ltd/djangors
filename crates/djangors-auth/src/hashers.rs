//! Pluggable password hashers and multi-hasher registry with transparent upgrade.
//!
//! Mirrors Django's `PASSWORD_HASHERS` architecture:
//! - [`Argon2Hasher`] (default, PHC `$argon2id$...`)
//! - [`Pbkdf2Sha256Hasher`] (exact Django format `pbkdf2_sha256$<iterations>$<salt>$<base64_hash>`)
//! - [`Pbkdf2Sha512Hasher`] (format `pbkdf2_sha512$<iterations>$<salt>$<base64_hash>`)
//! - [`PasswordHasherRegistry`] verifies any registered format and transparently
//!   re-hashes legacy hashes into the preferred algorithm on login.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher as _, PasswordVerifier as _};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand_core::RngCore;
use sha2::{Sha256, Sha512};
use subtle::ConstantTimeEq;

use crate::AuthError;

/// Trait implemented by pluggable password hashing algorithms.
pub trait PasswordHasher: Send + Sync {
    /// Short algorithm identifier (e.g. `"argon2"`, `"pbkdf2_sha256"`).
    fn algorithm(&self) -> &'static str;

    /// Encodes a plaintext `password` into a self-describing hash string.
    fn encode(&self, password: &str) -> Result<String, AuthError>;

    /// Verifies `password` against an `encoded` hash string produced by this algorithm.
    fn verify(&self, password: &str, encoded: &str) -> Result<bool, AuthError>;

    /// Returns `true` if this hasher recognizes the format of `encoded`.
    fn can_verify(&self, encoded: &str) -> bool;
}

/// Argon2id password hasher (default preferred algorithm, PHC `$argon2id$...` format).
#[derive(Debug, Default, Clone, Copy)]
pub struct Argon2Hasher;

impl PasswordHasher for Argon2Hasher {
    fn algorithm(&self) -> &'static str {
        "argon2"
    }

    fn encode(&self, password: &str) -> Result<String, AuthError> {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AuthError::Hashing(e.to_string()))?;
        Ok(hash.to_string())
    }

    fn verify(&self, password: &str, encoded: &str) -> Result<bool, AuthError> {
        let parsed = PasswordHash::new(encoded).map_err(|e| AuthError::Hashing(e.to_string()))?;
        Ok(Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok())
    }

    fn can_verify(&self, encoded: &str) -> bool {
        encoded.starts_with("$argon2id$")
            || encoded.starts_with("$argon2i$")
            || encoded.starts_with("$argon2d$")
    }
}

/// Pure-Rust PBKDF2-HMAC-SHA256 password hasher supporting the exact Django storage format:
/// `pbkdf2_sha256$<iterations>$<salt>$<base64_hash>`.
///
/// Enables zero-downtime migration of existing Django user databases into Djangors.
#[derive(Debug, Clone, Copy)]
pub struct Pbkdf2Sha256Hasher {
    iterations: u32,
}

impl Default for Pbkdf2Sha256Hasher {
    fn default() -> Self {
        Self {
            iterations: 600_000,
        }
    }
}

impl Pbkdf2Sha256Hasher {
    /// Creates a [`Pbkdf2Sha256Hasher`] with a custom iteration count (minimum 1).
    pub fn new(iterations: u32) -> Self {
        Self {
            iterations: iterations.max(1),
        }
    }

    /// Encodes `password` with an explicit `salt` and `iterations` in Django's format.
    pub fn encode_with_salt(&self, password: &str, salt: &str, iterations: u32) -> String {
        let dk = pbkdf2_hmac_sha256(password.as_bytes(), salt.as_bytes(), iterations.max(1), 32);
        let b64 = base64::engine::general_purpose::STANDARD.encode(dk);
        format!("pbkdf2_sha256${}${}${}", iterations.max(1), salt, b64)
    }
}

impl PasswordHasher for Pbkdf2Sha256Hasher {
    fn algorithm(&self) -> &'static str {
        "pbkdf2_sha256"
    }

    fn encode(&self, password: &str) -> Result<String, AuthError> {
        let salt = generate_django_salt(22);
        Ok(self.encode_with_salt(password, &salt, self.iterations))
    }

    fn verify(&self, password: &str, encoded: &str) -> Result<bool, AuthError> {
        let parts: Vec<&str> = encoded.split('$').collect();
        if parts.len() != 4 || parts[0] != "pbkdf2_sha256" {
            return Err(AuthError::Hashing(
                "invalid pbkdf2_sha256 hash format".to_string(),
            ));
        }
        let iterations: u32 = parts[1]
            .parse()
            .map_err(|_| AuthError::Hashing("invalid pbkdf2_sha256 iteration count".to_string()))?;
        if iterations == 0 {
            return Err(AuthError::Hashing(
                "pbkdf2_sha256 iterations must be > 0".to_string(),
            ));
        }
        let salt = parts[2];
        let expected = base64::engine::general_purpose::STANDARD
            .decode(parts[3])
            .map_err(|e| AuthError::Hashing(format!("invalid pbkdf2_sha256 base64: {e}")))?;

        let derived = pbkdf2_hmac_sha256(
            password.as_bytes(),
            salt.as_bytes(),
            iterations,
            expected.len(),
        );
        Ok(bool::from(derived.as_slice().ct_eq(expected.as_slice())))
    }

    fn can_verify(&self, encoded: &str) -> bool {
        encoded.starts_with("pbkdf2_sha256$")
    }
}

/// Pure-Rust PBKDF2-HMAC-SHA512 password hasher supporting the Django format:
/// `pbkdf2_sha512$<iterations>$<salt>$<base64_hash>`.
#[derive(Debug, Clone, Copy)]
pub struct Pbkdf2Sha512Hasher {
    iterations: u32,
}

impl Default for Pbkdf2Sha512Hasher {
    fn default() -> Self {
        Self {
            iterations: 210_000,
        }
    }
}

impl Pbkdf2Sha512Hasher {
    /// Creates a [`Pbkdf2Sha512Hasher`] with a custom iteration count (minimum 1).
    pub fn new(iterations: u32) -> Self {
        Self {
            iterations: iterations.max(1),
        }
    }

    /// Encodes `password` with an explicit `salt` and `iterations`.
    pub fn encode_with_salt(&self, password: &str, salt: &str, iterations: u32) -> String {
        let dk = pbkdf2_hmac_sha512(password.as_bytes(), salt.as_bytes(), iterations.max(1), 64);
        let b64 = base64::engine::general_purpose::STANDARD.encode(dk);
        format!("pbkdf2_sha512${}${}${}", iterations.max(1), salt, b64)
    }
}

impl PasswordHasher for Pbkdf2Sha512Hasher {
    fn algorithm(&self) -> &'static str {
        "pbkdf2_sha512"
    }

    fn encode(&self, password: &str) -> Result<String, AuthError> {
        let salt = generate_django_salt(22);
        Ok(self.encode_with_salt(password, &salt, self.iterations))
    }

    fn verify(&self, password: &str, encoded: &str) -> Result<bool, AuthError> {
        let parts: Vec<&str> = encoded.split('$').collect();
        if parts.len() != 4 || parts[0] != "pbkdf2_sha512" {
            return Err(AuthError::Hashing(
                "invalid pbkdf2_sha512 hash format".to_string(),
            ));
        }
        let iterations: u32 = parts[1]
            .parse()
            .map_err(|_| AuthError::Hashing("invalid pbkdf2_sha512 iteration count".to_string()))?;
        if iterations == 0 {
            return Err(AuthError::Hashing(
                "pbkdf2_sha512 iterations must be > 0".to_string(),
            ));
        }
        let salt = parts[2];
        let expected = base64::engine::general_purpose::STANDARD
            .decode(parts[3])
            .map_err(|e| AuthError::Hashing(format!("invalid pbkdf2_sha512 base64: {e}")))?;

        let derived = pbkdf2_hmac_sha512(
            password.as_bytes(),
            salt.as_bytes(),
            iterations,
            expected.len(),
        );
        Ok(bool::from(derived.as_slice().ct_eq(expected.as_slice())))
    }

    fn can_verify(&self, encoded: &str) -> bool {
        encoded.starts_with("pbkdf2_sha512$")
    }
}

/// Ordered registry of [`PasswordHasher`] implementations.
///
/// The first hasher is the preferred algorithm used for new hashes and upgrades.
/// Remaining hashers are accepted for verification of legacy hashes.
pub struct PasswordHasherRegistry {
    hashers: Vec<Box<dyn PasswordHasher>>,
}

impl Default for PasswordHasherRegistry {
    fn default() -> Self {
        Self {
            hashers: vec![
                Box::new(Argon2Hasher),
                Box::new(Pbkdf2Sha256Hasher::default()),
                Box::new(Pbkdf2Sha512Hasher::default()),
            ],
        }
    }
}

impl PasswordHasherRegistry {
    /// Creates a registry with a custom ordered list of hashers.
    ///
    /// The first entry is the preferred algorithm used by [`hash`](Self::hash) and
    /// [`verify_and_upgrade`](Self::verify_and_upgrade).
    pub fn new(hashers: Vec<Box<dyn PasswordHasher>>) -> Self {
        Self { hashers }
    }

    /// Hashes `password` using the preferred (first) hasher in the registry.
    pub fn hash(&self, password: &str) -> Result<String, AuthError> {
        let preferred = self
            .hashers
            .first()
            .ok_or_else(|| AuthError::Hashing("no password hashers configured".to_string()))?;
        preferred.encode(password)
    }

    /// Verifies `password` against `encoded` using whichever registered hasher
    /// recognizes the format.
    pub fn verify(&self, password: &str, encoded: &str) -> Result<bool, AuthError> {
        for hasher in &self.hashers {
            if hasher.can_verify(encoded) {
                return hasher.verify(password, encoded);
            }
        }
        Err(AuthError::Hashing(
            "unsupported or malformed password hash format".to_string(),
        ))
    }

    /// Returns `true` if `encoded` was not produced by the preferred (first) hasher
    /// and should be transparently upgraded on the next successful login.
    pub fn needs_rehash(&self, encoded: &str) -> bool {
        match self.hashers.first() {
            Some(preferred) => !preferred.can_verify(encoded),
            None => false,
        }
    }

    /// Verifies `password` against `encoded`, and if valid and [`needs_rehash`](Self::needs_rehash)
    /// is `true`, returns `(true, Some(upgraded_hash))`.
    pub fn verify_and_upgrade(
        &self,
        password: &str,
        encoded: &str,
    ) -> Result<(bool, Option<String>), AuthError> {
        let valid = self.verify(password, encoded)?;
        if valid && self.needs_rehash(encoded) {
            let upgraded = self.hash(password)?;
            Ok((true, Some(upgraded)))
        } else {
            Ok((valid, None))
        }
    }
}

fn generate_django_salt(len: usize) -> String {
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut bytes = vec![0u8; len];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .into_iter()
        .map(|b| CHARSET[(b as usize) % CHARSET.len()] as char)
        .collect()
}

fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32, dk_len: usize) -> Vec<u8> {
    pbkdf2_hmac::<Hmac<Sha256>, 32>(password, salt, iterations, dk_len)
}

fn pbkdf2_hmac_sha512(password: &[u8], salt: &[u8], iterations: u32, dk_len: usize) -> Vec<u8> {
    pbkdf2_hmac::<Hmac<Sha512>, 64>(password, salt, iterations, dk_len)
}

/// RFC 2898 PBKDF2-HMAC implementation generic over HMAC digest.
fn pbkdf2_hmac<M: Mac + hmac::digest::KeyInit, const BLOCK_LEN: usize>(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    dk_len: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(dk_len);
    let mut block_index: u32 = 1;

    while out.len() < dk_len {
        let mut mac = <M as Mac>::new_from_slice(password).expect("HMAC accepts any key length");
        mac.update(salt);
        mac.update(&block_index.to_be_bytes());
        let mut u = mac.finalize().into_bytes();
        let mut t = u.clone();

        for _ in 1..iterations {
            let mut mac =
                <M as Mac>::new_from_slice(password).expect("HMAC accepts any key length");
            mac.update(&u);
            u = mac.finalize().into_bytes();
            for (ti, ui) in t.iter_mut().zip(u.iter()) {
                *ti ^= *ui;
            }
        }

        let take = (dk_len - out.len()).min(BLOCK_LEN);
        out.extend_from_slice(&t[..take]);
        block_index = block_index.wrapping_add(1);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_sha256_matches_rfc6070_and_django_format() {
        // RFC 6070 / PBKDF2-HMAC-SHA256 test vector:
        // password="password", salt="salt", c=1 -> 120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b
        let dk = pbkdf2_hmac_sha256(b"password", b"salt", 1, 32);
        let hex: String = dk.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );

        let hasher = Pbkdf2Sha256Hasher::new(1_000);
        let encoded = hasher.encode("correct-horse-battery-staple").unwrap();
        assert!(encoded.starts_with("pbkdf2_sha256$1000$"));
        assert!(hasher.can_verify(&encoded));
        assert!(hasher
            .verify("correct-horse-battery-staple", &encoded)
            .unwrap());
        assert!(!hasher.verify("wrong-password", &encoded).unwrap());
    }

    #[test]
    fn pbkdf2_sha512_roundtrip() {
        let hasher = Pbkdf2Sha512Hasher::new(1_000);
        let encoded = hasher.encode("s3cr3t!").unwrap();
        assert!(encoded.starts_with("pbkdf2_sha512$1000$"));
        assert!(hasher.verify("s3cr3t!", &encoded).unwrap());
        assert!(!hasher.verify("nope", &encoded).unwrap());
    }

    #[test]
    fn registry_verifies_and_upgrades_legacy_django_hash() {
        let legacy_hasher = Pbkdf2Sha256Hasher::new(1_000);
        let legacy_hash = legacy_hasher.encode("my-legacy-password").unwrap();

        let registry = PasswordHasherRegistry::default();
        assert!(registry.needs_rehash(&legacy_hash));

        let (ok, upgraded) = registry
            .verify_and_upgrade("my-legacy-password", &legacy_hash)
            .unwrap();
        assert!(ok);
        let upgraded = upgraded.expect("legacy hash should be upgraded to argon2");
        assert!(upgraded.starts_with("$argon2id$"));
        assert!(!registry.needs_rehash(&upgraded));
        assert!(registry.verify("my-legacy-password", &upgraded).unwrap());

        let (bad_ok, bad_upgrade) = registry.verify_and_upgrade("wrong", &legacy_hash).unwrap();
        assert!(!bad_ok);
        assert!(bad_upgrade.is_none());
    }
}
