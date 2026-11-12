//! Distributed (and process-local SQLite) transaction-scoped advisory locks.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use sha2::{Digest, Sha256};

use crate::bind::BindValue;
use crate::database::{BoxFuture, Database};
use crate::dialect::Dialect;
use crate::error::DbError;
use crate::executor::Conn;

#[cfg(feature = "sqlite")]
static SQLITE_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(feature = "sqlite")]
fn sqlite_key_mutex(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = SQLITE_LOCKS.lock().expect("sqlite lock map poisoned");
    Arc::clone(
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// Derives a stable 64-bit advisory lock identifier from `key` using the first
/// 8 big-endian bytes of its SHA-256 digest.
pub(crate) fn advisory_lock_id(key: &str) -> i64 {
    let digest = Sha256::digest(key.as_bytes());
    let bytes: [u8; 8] = digest[..8].try_into().expect("sha256 digest is 32 bytes");
    i64::from_be_bytes(bytes)
}

/// Attempts to acquire a transaction-scoped lock for `key` without blocking,
/// running `f` inside a database transaction if acquired.
///
/// - **PostgreSQL**: runs `SELECT pg_try_advisory_xact_lock($1)` inside
///   `transaction_conn`; the lock is released automatically on commit or rollback.
///   Returns `Ok(None)` without running `f` if another session holds the lock.
/// - **SQLite**: uses a process-wide per-key `tokio::sync::Mutex` around
///   `transaction_conn`. Because SQLite is a single-process embedded database,
///   this covers all tasks within the process.
pub async fn with_lock<T, E, F>(db: &Database, key: &str, f: F) -> Result<Option<T>, DbError>
where
    F: for<'c> FnOnce(&'c mut Conn<'c>) -> BoxFuture<'c, Result<T, E>> + Send + 'static,
    T: Send,
    E: Into<DbError> + Send,
{
    match db.dialect() {
        #[cfg(feature = "postgres")]
        Dialect::Postgres => {
            let lock_id = advisory_lock_id(key);
            db.transaction_conn(|conn| {
                Box::pin(async move {
                    let row = conn
                        .fetch_one(
                            "SELECT pg_try_advisory_xact_lock($1)",
                            &[BindValue::I64(lock_id)],
                        )
                        .await
                        .map_err(DbError::QueryFailed)?;
                    let acquired = row
                        .try_bool(0)
                        .map_err(DbError::QueryFailed)?
                        .unwrap_or(false);
                    if !acquired {
                        return Ok::<Option<T>, DbError>(None);
                    }
                    let val = f(conn).await.map_err(Into::into)?;
                    Ok(Some(val))
                })
            })
            .await
        }
        #[cfg(feature = "sqlite")]
        Dialect::Sqlite => {
            let mutex = sqlite_key_mutex(key);
            let Ok(_guard) = mutex.try_lock_owned() else {
                return Ok(None);
            };
            let val = db.transaction_conn(f).await?;
            Ok(Some(val))
        }
        #[allow(unreachable_patterns)]
        _ => Err(DbError::TransactionFailed(
            "database backend feature not enabled for with_lock".to_string(),
        )),
    }
}

/// Acquires a transaction-scoped lock for `key`, waiting until it becomes
/// available, and runs `f` inside a database transaction.
///
/// - **PostgreSQL**: runs `SELECT pg_advisory_xact_lock($1)` inside
///   `transaction_conn`; released automatically on commit or rollback.
/// - **SQLite**: awaits a process-wide per-key `tokio::sync::Mutex` around
///   `transaction_conn`.
pub async fn with_lock_wait<T, E, F>(db: &Database, key: &str, f: F) -> Result<T, DbError>
where
    F: for<'c> FnOnce(&'c mut Conn<'c>) -> BoxFuture<'c, Result<T, E>> + Send + 'static,
    T: Send,
    E: Into<DbError> + Send,
{
    match db.dialect() {
        #[cfg(feature = "postgres")]
        Dialect::Postgres => {
            let lock_id = advisory_lock_id(key);
            db.transaction_conn(|conn| {
                Box::pin(async move {
                    conn.execute(
                        "SELECT pg_advisory_xact_lock($1)",
                        &[BindValue::I64(lock_id)],
                    )
                    .await
                    .map_err(DbError::QueryFailed)?;
                    f(conn).await.map_err(Into::into)
                })
            })
            .await
        }
        #[cfg(feature = "sqlite")]
        Dialect::Sqlite => {
            let mutex = sqlite_key_mutex(key);
            let _guard = mutex.lock_owned().await;
            db.transaction_conn(f).await
        }
        #[allow(unreachable_patterns)]
        _ => Err(DbError::TransactionFailed(
            "database backend feature not enabled for with_lock_wait".to_string(),
        )),
    }
}

#[cfg(test)]
#[path = "locks_tests.rs"]
mod tests;
