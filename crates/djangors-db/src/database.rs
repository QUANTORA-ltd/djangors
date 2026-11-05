use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Executor, PgConnection, PgPool, SqlitePool};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::DatabaseConfig;
use crate::dialect::Dialect;
use crate::error::DbError;
use crate::executor::Conn;

/// A pinned, boxed future that sends across threads, used for async callbacks
/// that borrow the database connection reference.
#[doc(hidden)]
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Connection pool manager for Djangors backends (PostgreSQL or SQLite).
#[derive(Debug, Clone)]
pub enum Database {
    /// PostgreSQL connection pool wrapper.
    Pg {
        /// Underlying SQLx PostgreSQL pool.
        pool: PgPool,
        /// Recorded query counter.
        query_count: Arc<AtomicUsize>,
    },
    /// SQLite connection pool wrapper.
    Sqlite {
        /// Underlying SQLx SQLite pool.
        pool: SqlitePool,
        /// Recorded query counter.
        query_count: Arc<AtomicUsize>,
    },
}

/// Supported isolation levels for database transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// Read Committed isolation level (PostgreSQL default).
    ReadCommitted,
    /// Repeatable Read isolation level.
    RepeatableRead,
    /// Serializable isolation level.
    Serializable,
}

/// Helper function to map [`IsolationLevel`] to the corresponding SQL command.
#[doc(hidden)]
pub fn isolation_level_sql(level: IsolationLevel) -> &'static str {
    match level {
        IsolationLevel::ReadCommitted => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        IsolationLevel::RepeatableRead => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
        IsolationLevel::Serializable => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
    }
}

impl Database {
    /// Connect to the database using the given configuration, building a connection pool.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DbError> {
        let url = config.url.trim();
        let is_sqlite = matches!(Dialect::from_url(url), Dialect::Sqlite);

        if is_sqlite {
            let mut options = SqlitePoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs));

            if let Some(idle) = config.idle_timeout_secs {
                options = options.idle_timeout(Duration::from_secs(idle));
            }

            let pool = options.connect(url).await.map_err(|e| {
                if matches!(e, sqlx::Error::PoolTimedOut) {
                    DbError::PoolExhausted
                } else {
                    DbError::ConnectionFailed(e.to_string())
                }
            })?;

            Ok(Self::Sqlite {
                pool,
                query_count: Arc::new(AtomicUsize::new(0)),
            })
        } else {
            let mut options = PgPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(Duration::from_secs(config.connect_timeout_secs));

            if let Some(idle) = config.idle_timeout_secs {
                options = options.idle_timeout(Duration::from_secs(idle));
            }

            let pool = options.connect(url).await.map_err(|e| {
                if matches!(e, sqlx::Error::PoolTimedOut) {
                    DbError::PoolExhausted
                } else {
                    DbError::ConnectionFailed(e.to_string())
                }
            })?;

            Ok(Self::Pg {
                pool,
                query_count: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    /// Access the underlying PostgreSQL connection pool directly.
    ///
    /// # Panics
    /// Panics if this `Database` handle is connected to a SQLite database.
    pub fn pool(&self) -> &PgPool {
        match self {
            Self::Pg { pool, .. } => pool,
            Self::Sqlite { .. } => {
                panic!("Attempted to access Postgres PgPool on a SQLite Database handle")
            }
        }
    }

    /// Access the underlying SQLite connection pool directly if connected to SQLite.
    pub fn sqlite_pool(&self) -> Option<&SqlitePool> {
        match self {
            Self::Sqlite { pool, .. } => Some(pool),
            Self::Pg { .. } => None,
        }
    }

    /// Returns a [`Conn`] execution handle for this database pool.
    pub fn conn(&self) -> Conn<'_> {
        match self {
            Self::Pg { pool, query_count } => Conn::traced_pg_pool(pool, query_count),
            Self::Sqlite { pool, query_count } => Conn::traced_sqlite_pool(pool, query_count),
        }
    }

    /// Configure the slow-query warning threshold for this database handle.
    ///
    /// Queries that take at least `threshold` emit a `WARN` tracing event with the
    /// redacted statement, inferred table, and elapsed time. Bound parameter values
    /// are never included. A zero duration causes every query to be reported.
    pub fn with_slow_query_threshold(self, threshold: Duration) -> Self {
        match &self {
            Self::Pg { query_count, .. } | Self::Sqlite { query_count, .. } => {
                crate::executor::set_slow_query_threshold(query_count, threshold);
            }
        }
        self
    }

    /// Attach a callback that receives timing and row-count data for each query.
    ///
    /// Applications can use this hook to export per-table counters and histograms
    /// through their preferred metrics system. Observer panics are caught so they
    /// cannot disrupt a database operation. Statements exclude bind values.
    pub fn with_query_observer<F>(self, observer: F) -> Self
    where
        F: Fn(&crate::executor::QueryEvent) + Send + Sync + 'static,
    {
        match &self {
            Self::Pg { query_count, .. } | Self::Sqlite { query_count, .. } => {
                crate::executor::set_query_observer(query_count, observer);
            }
        }
        self
    }

    /// Returns the database dialect.
    pub fn dialect(&self) -> Dialect {
        match self {
            Self::Pg { .. } => Dialect::Postgres,
            Self::Sqlite { .. } => Dialect::Sqlite,
        }
    }

    /// Records one SQL query for test observability.
    #[doc(hidden)]
    pub fn record_query(&self) {
        let counter = match self {
            Self::Pg { query_count, .. } => query_count,
            Self::Sqlite { query_count, .. } => query_count,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the number of SQL queries recorded by this database handle.
    #[doc(hidden)]
    pub fn query_count(&self) -> usize {
        let counter = match self {
            Self::Pg { query_count, .. } => query_count,
            Self::Sqlite { query_count, .. } => query_count,
        };
        counter.load(Ordering::Relaxed)
    }

    /// Resets the recorded SQL query count to zero.
    #[doc(hidden)]
    pub fn reset_query_count(&self) {
        let counter = match self {
            Self::Pg { query_count, .. } => query_count,
            Self::Sqlite { query_count, .. } => query_count,
        };
        counter.store(0, Ordering::Relaxed);
    }

    /// Run `f` inside a PostgreSQL database transaction.
    pub async fn transaction<F, T, E>(&self, f: F) -> Result<T, DbError>
    where
        F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, E>> + Send,
        T: Send,
        E: Into<DbError> + Send,
    {
        match self {
            Self::Pg { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let res = f(&mut tx).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
            Self::Sqlite { .. } => Err(DbError::TransactionFailed(
                "PostgreSQL PgConnection transaction called on a SQLite Database".to_string(),
            )),
        }
    }

    /// Run `f` inside a PostgreSQL database transaction with an explicit transaction isolation level.
    pub async fn transaction_with_isolation<F, T, E>(
        &self,
        level: IsolationLevel,
        f: F,
    ) -> Result<T, DbError>
    where
        F: for<'c> FnOnce(&'c mut PgConnection) -> BoxFuture<'c, Result<T, E>> + Send,
        T: Send,
        E: Into<DbError> + Send,
    {
        match self {
            Self::Pg { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let sql = isolation_level_sql(level);
                tx.execute(sql)
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;

                let res = f(&mut tx).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
            Self::Sqlite { .. } => Err(DbError::TransactionFailed(
                "PostgreSQL transaction_with_isolation called on a SQLite Database".to_string(),
            )),
        }
    }

    /// Run `f` inside a transaction on either backend.
    pub async fn transaction_conn<F, T, E>(&self, f: F) -> Result<T, DbError>
    where
        F: for<'c> FnOnce(&'c mut Conn<'c>) -> BoxFuture<'c, Result<T, E>> + Send,
        T: Send,
        E: Into<DbError> + Send,
    {
        match self {
            Self::Pg { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let mut conn = Conn::traced_pg_tx(
                    &mut tx,
                    match self {
                        Self::Pg { query_count, .. } => query_count,
                        _ => unreachable!(),
                    },
                );
                let res = f(&mut conn).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
            Self::Sqlite { pool, .. } => {
                let mut tx = pool
                    .begin()
                    .await
                    .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                let mut conn = Conn::traced_sqlite_tx(
                    &mut tx,
                    match self {
                        Self::Sqlite { query_count, .. } => query_count,
                        _ => unreachable!(),
                    },
                );
                let res = f(&mut conn).await;
                match res {
                    Ok(val) => {
                        tx.commit()
                            .await
                            .map_err(|e| DbError::TransactionFailed(e.to_string()))?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = tx.rollback().await;
                        Err(err.into())
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "database_tests.rs"]
mod tests;
