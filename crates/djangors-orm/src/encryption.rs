//! Transparent encryption for `#[djangors(encrypted)]` text fields.
//!
//! New encrypted values are stored as `enc:v1:<key_id>:<b64>` where `<key_id>`
//! is an 8-hex-character key fingerprint and `<b64>` is the standard base64 of
//! `nonce || ciphertext` (12-byte random nonce + AES-256-GCM ciphertext with
//! 16-byte authentication tag).
//!
//! Legacy `enc:v1:<b64>` values (without a key ID) remain readable with key 0 and
//! older configured keys, and values without an `enc:` prefix are returned as
//! legacy plaintext.
//!
//! Keys are loaded from the environment (newest first):
//!
//! - `DJANGORS_FIELD_ENCRYPTION_KEYS`: comma-separated base64-encoded 32-byte
//!   keys, newest first. The first key is always used for new writes.
//! - Legacy fallback when `DJANGORS_FIELD_ENCRYPTION_KEYS` is unset or empty:
//!   `DJANGORS_FIELD_ENCRYPTION_KEY` (current/first key) plus optional
//!   `DJANGORS_FIELD_ENCRYPTION_OLD_KEYS` (comma-separated older keys).

use crate::error::OrmError;
use crate::expr::Value;
use crate::meta::{all_registered_models, EncryptedFieldsRegistration, Model, ModelMeta};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::Aes256Gcm;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use djangors_db::{BindValue, Database, DbRow};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

const PREFIX: &str = "enc:v1:";
const PREFIX_V2: &str = "enc:v2:";
const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;
const KEYS_ENV: &str = "DJANGORS_FIELD_ENCRYPTION_KEYS";
const KEY_ENV: &str = "DJANGORS_FIELD_ENCRYPTION_KEY";
const OLD_KEYS_ENV: &str = "DJANGORS_FIELD_ENCRYPTION_OLD_KEYS";

#[derive(Clone)]
struct KeyEntry {
    id: String,
    raw: [u8; KEY_LEN],
}

/// Ordered ring of AES-256-GCM field encryption keys, newest first.
///
/// The first key encrypts all new writes; decryption uses the key named by the
/// ciphertext's key ID (or tries keys in order for legacy `enc:v1:<b64>`).
#[derive(Clone)]
pub struct KeyRing {
    keys: Vec<KeyEntry>,
}

/// The loaded keys, or the message explaining why they could not be loaded.
static KEY_RING: OnceLock<Result<KeyRing, String>> = OnceLock::new();

fn derive_key_id(key: &[u8; KEY_LEN]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"djangors-field-encryption-key-id-v1:");
    hasher.update(key);
    let digest = hasher.finalize();
    format!(
        "{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

impl KeyRing {
    /// Builds a [`KeyRing`] from raw 32-byte keys ordered newest-first.
    pub fn from_keys(keys: Vec<[u8; KEY_LEN]>) -> Result<Self, OrmError> {
        if keys.is_empty() {
            return Err(OrmError::InvalidQuery(
                "at least one 32-byte field encryption key is required".to_string(),
            ));
        }
        let entries = keys
            .into_iter()
            .map(|raw| KeyEntry {
                id: derive_key_id(&raw),
                raw,
            })
            .collect();
        Ok(Self { keys: entries })
    }

    /// Parses a comma-separated list of base64-encoded 32-byte keys (newest first).
    pub fn from_comma_separated(raw: &str) -> Result<Self, OrmError> {
        let mut parsed = Vec::new();
        for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let key = parse_key(part).map_err(OrmError::InvalidQuery)?;
            parsed.push(key);
        }
        if parsed.is_empty() {
            return Err(OrmError::InvalidQuery(format!(
                "{KEYS_ENV} must contain at least one base64-encoded 32-byte key"
            )));
        }
        Self::from_keys(parsed)
    }

    /// Loads keys from the environment (`DJANGORS_FIELD_ENCRYPTION_KEYS`, or
    /// `DJANGORS_FIELD_ENCRYPTION_KEY` + `DJANGORS_FIELD_ENCRYPTION_OLD_KEYS`).
    pub fn from_env() -> Result<Self, OrmError> {
        load_from_env().map_err(OrmError::InvalidQuery)
    }

    /// Returns the deterministic 8-hex key ID of the active (first) key.
    pub fn current_key_id(&self) -> &str {
        &self.keys[0].id
    }

    /// Returns the number of keys currently loaded in this ring.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Always `false` since a valid [`KeyRing`] contains at least one key.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Returns `true` if `stored` is not encrypted with the ring's current (first) key.
    ///
    /// Legacy plaintext values, legacy `enc:v1:<b64>` values without a key ID,
    /// `enc:v2:...` values, and ciphertexts tagged with an older key ID all return `true`.
    pub fn needs_rotation(&self, stored: &str) -> bool {
        let current_prefix = format!("{PREFIX}{}:", self.current_key_id());
        !stored.starts_with(&current_prefix)
    }
}

fn key_ring() -> Result<&'static KeyRing, OrmError> {
    let loaded = KEY_RING.get_or_init(load_from_env);
    match loaded {
        Ok(ring) => Ok(ring),
        Err(message) => Err(OrmError::InvalidQuery(message.clone())),
    }
}

fn load_from_env() -> Result<KeyRing, String> {
    if let Ok(raw_list) = std::env::var(KEYS_ENV) {
        if !raw_list.trim().is_empty() {
            return KeyRing::from_comma_separated(&raw_list).map_err(|e| e.to_string());
        }
    }

    let current_raw = std::env::var(KEY_ENV).map_err(|_| {
        format!(
            "field encryption key is not set: configure {KEYS_ENV} or {KEY_ENV} with base64-encoded 32-byte key(s)"
        )
    })?;
    let current = parse_key(current_raw.trim())?;
    let mut keys = vec![current];
    if let Ok(raw) = std::env::var(OLD_KEYS_ENV) {
        for part in raw
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            keys.push(parse_key(part)?);
        }
    }
    KeyRing::from_keys(keys).map_err(|e| e.to_string())
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
    let mut all = Vec::with_capacity(1 + old.len());
    all.push(current);
    all.extend(old);
    if let Ok(ring) = KeyRing::from_keys(all) {
        let _ = KEY_RING.set(Ok(ring));
    }
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

/// Encrypts `plain` with the first key in `ring` and returns the key-tagged
/// `enc:v1:<key_id>:<b64>` representation.
pub fn encrypt_str_with_ring(ring: &KeyRing, plain: &str) -> Result<String, OrmError> {
    let current = &ring.keys[0];
    let blob = seal(&current.raw, plain.as_bytes())?;
    Ok(format!("{PREFIX}{}:{}", current.id, STANDARD.encode(blob)))
}

/// Decrypts a stored field value against the supplied [`KeyRing`].
///
/// Values without `enc:v1:` or `enc:v2:` are returned unchanged as legacy plaintext.
/// Key-tagged values (`enc:v1:<key_id>:<b64>` or `enc:v2:<key_id>:<b64>`) decrypt
/// strictly with the matching key ID, returning a clear error if that key ID is
/// unknown or fails authentication. Legacy `enc:v1:<b64>` values without a key ID
/// try key 0 and then remaining keys in order.
pub fn decrypt_str_with_ring(ring: &KeyRing, stored: &str) -> Result<String, OrmError> {
    let (explicit_key_id, encoded) = if let Some(rest) = stored.strip_prefix(PREFIX) {
        match rest.split_once(':') {
            Some((key_id, b64)) => (Some(key_id), b64),
            None => (None, rest),
        }
    } else if let Some(rest) = stored.strip_prefix(PREFIX_V2) {
        let (key_id, b64) = rest.split_once(':').ok_or_else(|| {
            OrmError::InvalidQuery(
                "malformed v2 encrypted field value (missing key id)".to_string(),
            )
        })?;
        (Some(key_id), b64)
    } else {
        return Ok(stored.to_string());
    };

    let blob = STANDARD.decode(encoded).map_err(|_| {
        OrmError::InvalidQuery("encrypted field value is not valid base64".to_string())
    })?;

    let plain = match explicit_key_id {
        Some(key_id) => {
            let matching: Vec<&KeyEntry> = ring.keys.iter().filter(|k| k.id == key_id).collect();
            if matching.is_empty() {
                return Err(OrmError::InvalidQuery(format!(
                    "encrypted field references unknown key id '{key_id}'; ensure it is included in {KEYS_ENV} or {OLD_KEYS_ENV}"
                )));
            }
            matching
                .into_iter()
                .find_map(|entry| open(&entry.raw, &blob))
                .ok_or_else(|| {
                    OrmError::InvalidQuery(format!(
                        "encrypted field could not be decrypted with key id '{key_id}'"
                    ))
                })?
        }
        None => ring
            .keys
            .iter()
            .find_map(|entry| open(&entry.raw, &blob))
            .ok_or_else(|| {
                OrmError::InvalidQuery(
                    "encrypted field could not be decrypted with any configured key".to_string(),
                )
            })?,
    };

    String::from_utf8(plain)
        .map_err(|_| OrmError::InvalidQuery("decrypted field value is not valid UTF-8".to_string()))
}

/// Encrypts `plain` with the current key and returns the key-tagged `enc:v1:<key_id>:<b64>` form.
///
/// Every call uses a fresh random nonce, so encrypting the same text twice gives
/// different output.
pub fn encrypt_str(plain: &str) -> Result<String, OrmError> {
    let ring = key_ring()?;
    encrypt_str_with_ring(ring, plain)
}

/// Decrypts a stored field value.
///
/// Values without the `enc:v1:` (or `enc:v2:`) prefix are legacy plaintext and are
/// returned unchanged, so a column can be migrated to encryption in place.
/// Key-tagged values look up their key by ID; legacy `enc:v1:<b64>` values are
/// tried with the current key and then each old key. If none authenticates, an
/// error is returned; ciphertext is never returned as text.
pub fn decrypt_str(stored: &str) -> Result<String, OrmError> {
    let ring = key_ring()?;
    decrypt_str_with_ring(ring, stored)
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

/// Summary returned after rotating encrypted columns in a model table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRotationReport {
    /// Target database table name.
    pub table_name: String,
    /// Total rows inspected across all batches.
    pub rows_scanned: u64,
    /// Number of rows updated with new ciphertexts.
    pub rows_updated: u64,
    /// Number of transactional batches executed.
    pub batches: u64,
}

fn decode_pk_bind(row: &DbRow) -> Result<BindValue, OrmError> {
    if let Ok(Some(id)) = row.try_i64(0) {
        return Ok(BindValue::I64(id));
    }
    if let Ok(Some(s)) = row.try_string(0) {
        return Ok(BindValue::Text(s));
    }
    Err(OrmError::InvalidQuery(
        "encryption key rotation requires a non-null integer or text primary key".to_string(),
    ))
}

/// Re-encrypts `encrypted_columns` in `table_name` in transactional batches of
/// `batch_size` rows using the configured environment keys.
///
/// Rows whose encrypted columns already carry the active key ID are skipped,
/// making the operation safe to resume after an interruption.
pub async fn rotate_table_keys(
    db: &Database,
    table_name: &str,
    pk_column: &str,
    encrypted_columns: &[&str],
    batch_size: usize,
) -> Result<KeyRotationReport, OrmError> {
    let ring = key_ring()?;
    rotate_table_keys_with_ring(
        db,
        ring,
        table_name,
        pk_column,
        encrypted_columns,
        batch_size,
    )
    .await
}

/// Re-encrypts `encrypted_columns` in `table_name` inside transactional batches
/// of `batch_size` rows using the supplied [`KeyRing`].
///
/// Each batch commits independently; if any row cannot be decrypted with the
/// ring, its batch rolls back and returns an [`OrmError`].
pub async fn rotate_table_keys_with_ring(
    db: &Database,
    ring: &KeyRing,
    table_name: &str,
    pk_column: &str,
    encrypted_columns: &[&str],
    batch_size: usize,
) -> Result<KeyRotationReport, OrmError> {
    if batch_size == 0 {
        return Err(OrmError::InvalidQuery(
            "rotation batch size must be at least 1".to_string(),
        ));
    }
    if encrypted_columns.is_empty() {
        return Ok(KeyRotationReport {
            table_name: table_name.to_string(),
            rows_scanned: 0,
            rows_updated: 0,
            batches: 0,
        });
    }

    let batch_limit = i64::try_from(batch_size)
        .map_err(|_| OrmError::InvalidQuery("batch size exceeds i64 range".to_string()))?;
    let dialect = db.dialect();
    let table_quoted = dialect.quote_ident(table_name);
    let pk_quoted = dialect.quote_ident(pk_column);
    let col_names: Vec<String> = encrypted_columns.iter().map(|c| (*c).to_string()).collect();
    let cols_select: String = std::iter::once(pk_quoted.clone())
        .chain(col_names.iter().map(|c| dialect.quote_ident(c)))
        .collect::<Vec<_>>()
        .join(", ");

    let mut report = KeyRotationReport {
        table_name: table_name.to_string(),
        rows_scanned: 0,
        rows_updated: 0,
        batches: 0,
    };
    let mut cursor: Option<BindValue> = None;

    loop {
        let cursor_arg = cursor.clone();
        let batch_ring = ring.clone();
        let batch_cols = col_names.clone();
        let table_q = table_quoted.clone();
        let pk_q = pk_quoted.clone();
        let select_cols = cols_select.clone();

        let (scanned, updated, next_cursor) = db
            .transaction_conn(|conn| {
                Box::pin(async move {
                    let (select_sql, select_params): (String, Vec<BindValue>) = match &cursor_arg {
                        Some(pk_val) => (
                            format!(
                                "SELECT {select_cols} FROM {table_q} WHERE {pk_q} > {} ORDER BY {pk_q} ASC LIMIT {}",
                                dialect.placeholder(1),
                                dialect.placeholder(2),
                            ),
                            vec![pk_val.clone(), BindValue::I64(batch_limit)],
                        ),
                        None => (
                            format!(
                                "SELECT {select_cols} FROM {table_q} ORDER BY {pk_q} ASC LIMIT {}",
                                dialect.placeholder(1),
                            ),
                            vec![BindValue::I64(batch_limit)],
                        ),
                    };

                    let rows = conn
                        .fetch_all(&select_sql, &select_params)
                        .await
                        .map_err(OrmError::Query)?;
                    if rows.is_empty() {
                        return Ok::<(u64, u64, Option<BindValue>), OrmError>((0, 0, None));
                    }

                    let mut last_pk = None;
                    let mut updated_count: u64 = 0;

                    for row in &rows {
                        let pk_val = decode_pk_bind(row)?;
                        let mut assignments: Vec<String> = Vec::new();
                        let mut params: Vec<BindValue> = Vec::new();

                        for (idx, col_name) in batch_cols.iter().enumerate() {
                            let stored = row.try_string(idx + 1).map_err(OrmError::Query)?;
                            if let Some(raw) = stored {
                                if batch_ring.needs_rotation(&raw) {
                                    let plain = decrypt_str_with_ring(&batch_ring, &raw)?;
                                    let reencrypted = encrypt_str_with_ring(&batch_ring, &plain)?;
                                    let param_idx = params.len() + 1;
                                    assignments.push(format!(
                                        "{} = {}",
                                        dialect.quote_ident(col_name),
                                        dialect.placeholder(param_idx)
                                    ));
                                    params.push(BindValue::Text(reencrypted));
                                }
                            }
                        }

                        if !assignments.is_empty() {
                            let pk_param_idx = params.len() + 1;
                            params.push(pk_val.clone());
                            let update_sql = format!(
                                "UPDATE {table_q} SET {} WHERE {pk_q} = {}",
                                assignments.join(", "),
                                dialect.placeholder(pk_param_idx),
                            );
                            conn.execute(&update_sql, &params)
                                .await
                                .map_err(OrmError::Query)?;
                            updated_count += 1;
                        }

                        last_pk = Some(pk_val);
                    }

                    Ok((rows.len() as u64, updated_count, last_pk))
                })
            })
            .await
            .map_err(|e| OrmError::InvalidQuery(e.to_string()))?;

        if scanned == 0 {
            break;
        }
        report.rows_scanned += scanned;
        report.rows_updated += updated;
        report.batches += 1;
        cursor = next_cursor;
        if scanned < batch_size as u64 {
            break;
        }
    }

    Ok(report)
}

fn resolve_model_columns(
    meta: &'static ModelMeta,
    encrypted_field_names: &[&'static str],
) -> Result<(&'static str, Vec<&'static str>), OrmError> {
    let pk_col = meta
        .fields
        .iter()
        .find(|f| f.primary_key)
        .map(|f| f.column_name)
        .ok_or_else(|| {
            OrmError::InvalidQuery(format!(
                "model '{}' does not have a primary key field",
                meta.struct_name
            ))
        })?;
    let cols = encrypted_field_names
        .iter()
        .map(|field_name| {
            meta.fields
                .iter()
                .find(|f| f.name == *field_name)
                .map(|f| f.column_name)
                .unwrap_or(field_name)
        })
        .collect();
    Ok((pk_col, cols))
}

/// Re-encrypts all `#[djangors(encrypted)]` fields on model `M` using the active [`KeyRing`].
pub async fn rotate_model_keys<M: Model>(
    db: &Database,
    batch_size: usize,
) -> Result<KeyRotationReport, OrmError> {
    let ring = key_ring()?;
    rotate_model_keys_with_ring::<M>(db, ring, batch_size).await
}

/// Re-encrypts all `#[djangors(encrypted)]` fields on model `M` using `ring`.
pub async fn rotate_model_keys_with_ring<M: Model>(
    db: &Database,
    ring: &KeyRing,
    batch_size: usize,
) -> Result<KeyRotationReport, OrmError> {
    let meta = M::meta();
    let (pk_col, cols) = resolve_model_columns(meta, M::encrypted_fields())?;
    rotate_table_keys_with_ring(db, ring, meta.table_name, pk_col, &cols, batch_size).await
}

/// Looks up a compiled `#[derive(Model)]` by `app.Model` (e.g. `"billing.Card"`)
/// or `Model` struct name, and rotates all of its `#[djangors(encrypted)]` fields.
pub async fn rotate_registered_model_keys(
    db: &Database,
    model_spec: &str,
    batch_size: usize,
) -> Result<KeyRotationReport, OrmError> {
    let ring = KeyRing::from_env()?;
    rotate_registered_model_keys_with_ring(db, &ring, model_spec, batch_size).await
}

/// Variant of [`rotate_registered_model_keys`] taking an explicit [`KeyRing`].
pub async fn rotate_registered_model_keys_with_ring(
    db: &Database,
    ring: &KeyRing,
    model_spec: &str,
    batch_size: usize,
) -> Result<KeyRotationReport, OrmError> {
    let spec = model_spec.trim();
    let meta = if let Some((app, model_name)) = spec.split_once('.') {
        all_registered_models()
            .find(|m| {
                m.app_label.eq_ignore_ascii_case(app)
                    && (m.struct_name.eq_ignore_ascii_case(model_name)
                        || m.table_name.eq_ignore_ascii_case(model_name))
            })
            .ok_or_else(|| {
                OrmError::InvalidQuery(format!("unknown model '{model_spec}' in model registry"))
            })?
    } else {
        all_registered_models()
            .find(|m| m.struct_name.eq_ignore_ascii_case(spec))
            .ok_or_else(|| {
                OrmError::InvalidQuery(format!("unknown model '{model_spec}' in model registry"))
            })?
    };

    let encrypted_field_names: &'static [&'static str] =
        inventory::iter::<EncryptedFieldsRegistration>()
            .find(|reg| reg.struct_name == meta.struct_name)
            .map(|reg| (reg.fields_fn)())
            .unwrap_or(&[]);

    let (pk_col, cols) = resolve_model_columns(meta, encrypted_field_names)?;
    rotate_table_keys_with_ring(db, ring, meta.table_name, pk_col, &cols, batch_size).await
}

#[cfg(test)]
#[path = "encryption_tests.rs"]
mod tests;
