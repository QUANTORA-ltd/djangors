//! Abstraction over where a query runs: a connection pool, or an open
//! transaction.

#[cfg(feature = "postgres")]
use sqlx::{PgConnection, PgPool};
#[cfg(feature = "sqlite")]
use sqlx::{SqliteConnection, SqlitePool};

use crate::bind::{BindValue, NullKind};
use crate::database::Database;
use crate::dialect::Dialect;
use crate::error::DbError;
use crate::row::DbRow;

/// A borrowed handle to whatever a query should run against.
pub enum Conn<'a> {
    /// Run against the PostgreSQL connection pool.
    #[cfg(feature = "postgres")]
    PgPool(&'a PgPool),
    /// Run against a single PostgreSQL connection in a transaction.
    #[cfg(feature = "postgres")]
    PgTx(&'a mut PgConnection),
    /// Run against the SQLite connection pool.
    #[cfg(feature = "sqlite")]
    SqlitePool(&'a SqlitePool),
    /// Run against a single SQLite connection in a transaction.
    #[cfg(feature = "sqlite")]
    SqliteTx(&'a mut SqliteConnection),
}

#[cfg(feature = "postgres")]
fn build_pg_query<'q>(
    sql: &'q str,
    params: &'q [BindValue],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for val in params {
        q = match val {
            BindValue::I64(v) => q.bind(*v),
            BindValue::F64(v) => q.bind(*v),
            BindValue::Text(v) => q.bind(v.as_str()),
            BindValue::Bool(v) => q.bind(*v),
            BindValue::DateTime(v) => q.bind(*v),
            BindValue::Bytes(v) => q.bind(v.as_slice()),
            BindValue::Null(kind) => match kind {
                NullKind::I64 => q.bind(None::<i64>),
                NullKind::F64 => q.bind(None::<f64>),
                NullKind::Text => q.bind(None::<String>),
                NullKind::Bool => q.bind(None::<bool>),
                NullKind::DateTime => q.bind(None::<chrono::DateTime<chrono::Utc>>),
                NullKind::Bytes => q.bind(None::<Vec<u8>>),
            },
        };
    }
    q
}

#[cfg(feature = "sqlite")]
fn build_sqlite_query<'q>(
    sql: &'q str,
    params: &'q [BindValue],
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql));
    for val in params {
        q = match val {
            BindValue::I64(v) => q.bind(*v),
            BindValue::F64(v) => q.bind(*v),
            BindValue::Text(v) => q.bind(v.as_str()),
            BindValue::Bool(v) => q.bind(*v),
            BindValue::DateTime(v) => q.bind(*v),
            BindValue::Bytes(v) => q.bind(v.as_slice()),
            BindValue::Null(kind) => match kind {
                NullKind::I64 => q.bind(None::<i64>),
                NullKind::F64 => q.bind(None::<f64>),
                NullKind::Text => q.bind(None::<String>),
                NullKind::Bool => q.bind(None::<bool>),
                NullKind::DateTime => q.bind(None::<chrono::DateTime<chrono::Utc>>),
                NullKind::Bytes => q.bind(None::<Vec<u8>>),
            },
        };
    }
    q
}

impl<'a> Conn<'a> {
    /// Execute the query and collect every row.
    pub async fn fetch_all(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<Vec<DbRow>, sqlx::Error> {
        let start = crate::query_log::is_logging_active().then(std::time::Instant::now);
        let res = match self {
            #[cfg(feature = "postgres")]
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let rows = q.fetch_all(*pool).await?;
                Ok(rows.into_iter().map(DbRow::Pg).collect())
            }
            #[cfg(feature = "postgres")]
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let rows = q.fetch_all(&mut **tx).await?;
                Ok(rows.into_iter().map(DbRow::Pg).collect())
            }
            #[cfg(feature = "sqlite")]
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let rows = q.fetch_all(*pool).await?;
                Ok(rows.into_iter().map(DbRow::Sqlite).collect())
            }
            #[cfg(feature = "sqlite")]
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let rows = q.fetch_all(&mut **tx).await?;
                Ok(rows.into_iter().map(DbRow::Sqlite).collect())
            }
        };
        if let Some(t0) = start {
            crate::query_log::record_query(sql, params.len(), t0.elapsed().as_micros() as u64);
        }
        res
    }

    /// Execute the query, requiring exactly one row.
    pub async fn fetch_one(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<DbRow, sqlx::Error> {
        let start = crate::query_log::is_logging_active().then(std::time::Instant::now);
        let res = match self {
            #[cfg(feature = "postgres")]
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_one(*pool).await?;
                Ok(DbRow::Pg(row))
            }
            #[cfg(feature = "postgres")]
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_one(&mut **tx).await?;
                Ok(DbRow::Pg(row))
            }
            #[cfg(feature = "sqlite")]
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_one(*pool).await?;
                Ok(DbRow::Sqlite(row))
            }
            #[cfg(feature = "sqlite")]
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_one(&mut **tx).await?;
                Ok(DbRow::Sqlite(row))
            }
        };
        if let Some(t0) = start {
            crate::query_log::record_query(sql, params.len(), t0.elapsed().as_micros() as u64);
        }
        res
    }

    /// Execute the query, returning the row if there is one.
    pub async fn fetch_optional(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<Option<DbRow>, sqlx::Error> {
        let start = crate::query_log::is_logging_active().then(std::time::Instant::now);
        let res = match self {
            #[cfg(feature = "postgres")]
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_optional(*pool).await?;
                Ok(row.map(DbRow::Pg))
            }
            #[cfg(feature = "postgres")]
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let row = q.fetch_optional(&mut **tx).await?;
                Ok(row.map(DbRow::Pg))
            }
            #[cfg(feature = "sqlite")]
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_optional(*pool).await?;
                Ok(row.map(DbRow::Sqlite))
            }
            #[cfg(feature = "sqlite")]
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let row = q.fetch_optional(&mut **tx).await?;
                Ok(row.map(DbRow::Sqlite))
            }
        };
        if let Some(t0) = start {
            crate::query_log::record_query(sql, params.len(), t0.elapsed().as_micros() as u64);
        }
        res
    }

    /// Execute the query for its side effect, returning the affected-row count.
    pub async fn execute(&mut self, sql: &str, params: &[BindValue]) -> Result<u64, sqlx::Error> {
        let start = crate::query_log::is_logging_active().then(std::time::Instant::now);
        let res = match self {
            #[cfg(feature = "postgres")]
            Conn::PgPool(pool) => {
                let q = build_pg_query(sql, params);
                let res = q.execute(*pool).await?;
                Ok(res.rows_affected())
            }
            #[cfg(feature = "postgres")]
            Conn::PgTx(tx) => {
                let q = build_pg_query(sql, params);
                let res = q.execute(&mut **tx).await?;
                Ok(res.rows_affected())
            }
            #[cfg(feature = "sqlite")]
            Conn::SqlitePool(pool) => {
                let q = build_sqlite_query(sql, params);
                let res = q.execute(*pool).await?;
                Ok(res.rows_affected())
            }
            #[cfg(feature = "sqlite")]
            Conn::SqliteTx(tx) => {
                let q = build_sqlite_query(sql, params);
                let res = q.execute(&mut **tx).await?;
                Ok(res.rows_affected())
            }
        };
        if let Some(t0) = start {
            crate::query_log::record_query(sql, params.len(), t0.elapsed().as_micros() as u64);
        }
        res
    }

    /// The database dialect for this connection handle.
    pub fn dialect(&self) -> Dialect {
        match self {
            #[cfg(feature = "postgres")]
            Conn::PgPool(_) | Conn::PgTx(_) => Dialect::Postgres,
            #[cfg(feature = "sqlite")]
            Conn::SqlitePool(_) | Conn::SqliteTx(_) => Dialect::Sqlite,
        }
    }

    /// Whether this handle is inside an open transaction.
    pub fn in_transaction(&self) -> bool {
        match self {
            #[cfg(feature = "postgres")]
            Conn::PgTx(_) => true,
            #[cfg(feature = "sqlite")]
            Conn::SqliteTx(_) => true,
            _ => false,
        }
    }

    /// Establishes a new savepoint with the given name inside the current transaction.
    pub async fn savepoint(&mut self, name: &str) -> Result<(), DbError> {
        if !self.in_transaction() {
            return Err(DbError::SavepointOutsideTransaction);
        }
        validate_savepoint_name(name)?;
        let quoted = self.dialect().quote_ident(name);
        let sql = format!("SAVEPOINT {}", quoted);
        self.execute(&sql, &[]).await?;
        Ok(())
    }

    /// Rolls back the transaction to the specified savepoint.
    pub async fn rollback_to_savepoint(&mut self, name: &str) -> Result<(), DbError> {
        if !self.in_transaction() {
            return Err(DbError::SavepointOutsideTransaction);
        }
        validate_savepoint_name(name)?;
        let quoted = self.dialect().quote_ident(name);
        let sql = format!("ROLLBACK TO SAVEPOINT {}", quoted);
        self.execute(&sql, &[]).await?;
        Ok(())
    }

    /// Releases (destroys) the specified savepoint.
    pub async fn release_savepoint(&mut self, name: &str) -> Result<(), DbError> {
        if !self.in_transaction() {
            return Err(DbError::SavepointOutsideTransaction);
        }
        validate_savepoint_name(name)?;
        let quoted = self.dialect().quote_ident(name);
        let sql = format!("RELEASE SAVEPOINT {}", quoted);
        self.execute(&sql, &[]).await?;
        Ok(())
    }
}

fn validate_savepoint_name(name: &str) -> Result<(), DbError> {
    if name.is_empty() || name.len() > 63 {
        return Err(DbError::InvalidSavepointName(name.to_string()));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(DbError::InvalidSavepointName(name.to_string()));
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' {
            return Err(DbError::InvalidSavepointName(name.to_string()));
        }
    }
    Ok(())
}

/// A target the ORM can run queries against.
pub trait DbExecutor {
    /// Borrow the underlying execution target.
    fn conn(&mut self) -> Conn<'_>;

    /// Returns the database dialect.
    fn dialect(&mut self) -> Dialect {
        self.conn().dialect()
    }

    /// Record one query for test observability.
    fn record_query(&self) {}
}

impl DbExecutor for &Database {
    fn conn(&mut self) -> Conn<'_> {
        Database::conn(self)
    }

    fn record_query(&self) {
        Database::record_query(self);
    }
}

#[cfg(feature = "postgres")]
impl DbExecutor for &mut PgConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::PgTx(self)
    }
}

#[cfg(feature = "sqlite")]
impl DbExecutor for &mut SqliteConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::SqliteTx(self)
    }
}

impl DbExecutor for Conn<'_> {
    fn conn(&mut self) -> Conn<'_> {
        match self {
            #[cfg(feature = "postgres")]
            Conn::PgPool(pool) => Conn::PgPool(pool),
            #[cfg(feature = "postgres")]
            Conn::PgTx(tx) => Conn::PgTx(tx),
            #[cfg(feature = "sqlite")]
            Conn::SqlitePool(pool) => Conn::SqlitePool(pool),
            #[cfg(feature = "sqlite")]
            Conn::SqliteTx(tx) => Conn::SqliteTx(tx),
        }
    }
}
