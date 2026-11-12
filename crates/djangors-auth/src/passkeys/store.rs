//! Database storage for passkeys (`djangors_auth_passkey`).

use super::{Passkey, PasskeyError};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use djangors_db::{BindValue, Database, DbRow};

/// Stores passkeys in the `djangors_auth_passkey` table, created on first use.
/// Binary values are stored as base64url text so the table is identical on
/// PostgreSQL and SQLite.
#[derive(Clone)]
pub struct PasskeyStore {
    db: Database,
}

fn db_err(err: impl std::fmt::Display) -> PasskeyError {
    PasskeyError::Database(err.to_string())
}

impl PasskeyStore {
    /// Creates a store backed by `db`.
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Creates the table and its indexes if they don't exist.
    pub async fn ensure_table(&self) -> Result<(), PasskeyError> {
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let create = format!(
            "CREATE TABLE IF NOT EXISTS djangors_auth_passkey (\
             id {pk}, \
             user_id BIGINT NOT NULL, \
             credential_id TEXT NOT NULL UNIQUE, \
             public_key TEXT NOT NULL, \
             sign_count BIGINT NOT NULL, \
             user_handle TEXT NOT NULL, \
             backup_eligible BOOLEAN NOT NULL, \
             name TEXT NOT NULL, \
             created_at {ts} NOT NULL, \
             last_used_at {ts})",
            pk = dialect.auto_pk_type(),
            ts = dialect.timestamp_type(),
        );
        conn.execute(&create, &[]).await.map_err(db_err)?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS djangors_auth_passkey_user ON djangors_auth_passkey (user_id)",
            &[],
        )
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Saves a newly registered passkey for `user_id` under a display `name`
    /// (for example "MacBook" or "YubiKey").
    pub async fn add(
        &self,
        user_id: i64,
        passkey: &Passkey,
        name: &str,
    ) -> Result<(), PasskeyError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let d = conn.dialect();
        let sql = format!(
            "INSERT INTO djangors_auth_passkey \
             (user_id, credential_id, public_key, sign_count, user_handle, backup_eligible, name, created_at) \
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            d.placeholder(1),
            d.placeholder(2),
            d.placeholder(3),
            d.placeholder(4),
            d.placeholder(5),
            d.placeholder(6),
            d.placeholder(7),
            d.placeholder(8),
        );
        conn.execute(
            &sql,
            &[
                BindValue::I64(user_id),
                BindValue::Text(URL_SAFE_NO_PAD.encode(&passkey.credential_id)),
                BindValue::Text(URL_SAFE_NO_PAD.encode(&passkey.public_key)),
                BindValue::I64(i64::from(passkey.sign_count)),
                BindValue::Text(URL_SAFE_NO_PAD.encode(&passkey.user_handle)),
                BindValue::Bool(passkey.backup_eligible),
                BindValue::Text(name.to_string()),
                BindValue::DateTime(chrono::Utc::now()),
            ],
        )
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Finds a passkey by credential ID, returning the owning user's ID with it.
    pub async fn find(&self, credential_id: &[u8]) -> Result<Option<(i64, Passkey)>, PasskeyError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let sql = format!(
            "SELECT user_id, credential_id, public_key, sign_count, user_handle, backup_eligible \
             FROM djangors_auth_passkey WHERE credential_id = {}",
            conn.dialect().placeholder(1)
        );
        let row = conn
            .fetch_optional(
                &sql,
                &[BindValue::Text(URL_SAFE_NO_PAD.encode(credential_id))],
            )
            .await
            .map_err(db_err)?;
        match row {
            Some(row) => {
                let user_id = required(row.try_i64_by_name("user_id"), "user_id")?;
                Ok(Some((user_id, passkey_from_row(&row)?)))
            }
            None => Ok(None),
        }
    }

    /// All passkeys registered by a user, oldest first.
    pub async fn for_user(&self, user_id: i64) -> Result<Vec<Passkey>, PasskeyError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let sql = format!(
            "SELECT credential_id, public_key, sign_count, user_handle, backup_eligible \
             FROM djangors_auth_passkey WHERE user_id = {} ORDER BY id",
            conn.dialect().placeholder(1)
        );
        let rows = conn
            .fetch_all(&sql, &[BindValue::I64(user_id)])
            .await
            .map_err(db_err)?;
        rows.iter().map(passkey_from_row).collect()
    }

    /// Records a successful sign-in: saves the new signature counter and the time.
    pub async fn update_sign_count(
        &self,
        credential_id: &[u8],
        sign_count: u32,
    ) -> Result<(), PasskeyError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let d = conn.dialect();
        let sql = format!(
            "UPDATE djangors_auth_passkey SET sign_count = {}, last_used_at = {} WHERE credential_id = {}",
            d.placeholder(1),
            d.placeholder(2),
            d.placeholder(3),
        );
        conn.execute(
            &sql,
            &[
                BindValue::I64(i64::from(sign_count)),
                BindValue::DateTime(chrono::Utc::now()),
                BindValue::Text(URL_SAFE_NO_PAD.encode(credential_id)),
            ],
        )
        .await
        .map_err(db_err)?;
        Ok(())
    }

    /// Removes a passkey. Only deletes it if it belongs to `user_id`, so a user can't
    /// remove someone else's passkey by guessing its ID. Returns whether one was removed.
    pub async fn remove(&self, user_id: i64, credential_id: &[u8]) -> Result<bool, PasskeyError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let d = conn.dialect();
        let sql = format!(
            "DELETE FROM djangors_auth_passkey WHERE user_id = {} AND credential_id = {}",
            d.placeholder(1),
            d.placeholder(2),
        );
        let removed = conn
            .execute(
                &sql,
                &[
                    BindValue::I64(user_id),
                    BindValue::Text(URL_SAFE_NO_PAD.encode(credential_id)),
                ],
            )
            .await
            .map_err(db_err)?;
        Ok(removed > 0)
    }
}

fn required<T, E: std::fmt::Display>(
    value: Result<Option<T>, E>,
    column: &str,
) -> Result<T, PasskeyError> {
    value
        .map_err(db_err)?
        .ok_or_else(|| PasskeyError::Database(format!("column {column} is NULL")))
}

fn decode_column(row: &DbRow, column: &str) -> Result<Vec<u8>, PasskeyError> {
    let text = required(row.try_string_by_name(column), column)?;
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| PasskeyError::Database(format!("column {column} is not base64url")))
}

fn passkey_from_row(row: &DbRow) -> Result<Passkey, PasskeyError> {
    let sign_count = required(row.try_i64_by_name("sign_count"), "sign_count")?;
    Ok(Passkey {
        credential_id: decode_column(row, "credential_id")?,
        public_key: decode_column(row, "public_key")?,
        sign_count: u32::try_from(sign_count).map_err(db_err)?,
        user_handle: decode_column(row, "user_handle")?,
        backup_eligible: required(row.try_bool_by_name("backup_eligible"), "backup_eligible")?,
    })
}
