//! Abstraction over where a query runs: a connection pool, or an open
//! transaction.

use sqlx::{PgConnection, PgPool, SqliteConnection, SqlitePool};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tracing::Instrument;

use crate::bind::{BindValue, NullKind};
use crate::database::Database;
use crate::dialect::Dialect;
use crate::error::DbError;
use crate::row::DbRow;

/// A borrowed handle to whatever a query should run against.
pub enum Conn<'a> {
    /// Run against the PostgreSQL connection pool.
    PgPool(&'a PgPool),
    /// Run against a single PostgreSQL connection in a transaction.
    PgTx(&'a mut PgConnection),
    /// Run against the SQLite connection pool.
    SqlitePool(&'a SqlitePool),
    /// Run against a single SQLite connection in a transaction.
    SqliteTx(&'a mut SqliteConnection),
    /// Traced PostgreSQL pool connection used by [`Database`](crate::Database).
    #[doc(hidden)]
    TracedPgPool(&'a PgPool, Arc<QuerySettings>),
    /// Traced PostgreSQL transaction connection used by [`Database`](crate::Database).
    #[doc(hidden)]
    TracedPgTx(&'a mut PgConnection, Arc<QuerySettings>),
    /// Traced SQLite pool connection used by [`Database`](crate::Database).
    #[doc(hidden)]
    TracedSqlitePool(&'a SqlitePool, Arc<QuerySettings>),
    /// Traced SQLite transaction connection used by [`Database`](crate::Database).
    #[doc(hidden)]
    TracedSqliteTx(&'a mut SqliteConnection, Arc<QuerySettings>),
}

/// One completed database operation delivered to a configured query observer.
#[derive(Debug, Clone)]
pub struct QueryEvent {
    /// SQL statement with quoted string literals redacted.
    pub statement: String,
    /// Table name inferred from the statement's primary target, if recognized.
    pub model: Option<String>,
    /// Number of returned or affected rows, when available.
    pub rows: Option<u64>,
    /// Total elapsed wall time for the database operation.
    pub duration: Duration,
    /// Whether the operation completed successfully.
    pub success: bool,
    /// Whether the configured slow-query threshold was reached.
    pub slow: bool,
}

type QueryObserver = Arc<dyn Fn(&QueryEvent) + Send + Sync + 'static>;

/// Shared query instrumentation settings associated with one database handle.
/// Internal query instrumentation settings carried by traced [`Conn`] variants.
#[doc(hidden)]
pub struct QuerySettings {
    slow_threshold_nanos: AtomicU64,
    observer: Mutex<Option<QueryObserver>>,
}

impl QuerySettings {
    fn new() -> Self {
        Self {
            slow_threshold_nanos: AtomicU64::new(u64::MAX),
            observer: Mutex::new(None),
        }
    }
}

struct QuerySettingsEntry {
    counter: Weak<AtomicUsize>,
    settings: Arc<QuerySettings>,
}

static QUERY_SETTINGS: OnceLock<Mutex<HashMap<usize, QuerySettingsEntry>>> = OnceLock::new();

fn settings_for(counter: &Arc<AtomicUsize>) -> Arc<QuerySettings> {
    let registry = QUERY_SETTINGS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|_, entry| entry.counter.strong_count() > 0);
    let key = Arc::as_ptr(counter) as usize;
    registry
        .entry(key)
        .or_insert_with(|| QuerySettingsEntry {
            counter: Arc::downgrade(counter),
            settings: Arc::new(QuerySettings::new()),
        })
        .settings
        .clone()
}

pub(crate) fn set_slow_query_threshold(counter: &Arc<AtomicUsize>, threshold: Duration) {
    settings_for(counter).slow_threshold_nanos.store(
        threshold.as_nanos().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
}

pub(crate) fn set_query_observer<F>(counter: &Arc<AtomicUsize>, observer: F)
where
    F: Fn(&QueryEvent) + Send + Sync + 'static,
{
    *settings_for(counter)
        .observer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(observer));
}

impl<'a> Conn<'a> {
    pub(crate) fn traced_pg_pool(pool: &'a PgPool, counter: &Arc<AtomicUsize>) -> Self {
        Self::TracedPgPool(pool, settings_for(counter))
    }

    pub(crate) fn traced_sqlite_pool(pool: &'a SqlitePool, counter: &Arc<AtomicUsize>) -> Self {
        Self::TracedSqlitePool(pool, settings_for(counter))
    }

    pub(crate) fn traced_pg_tx(conn: &'a mut PgConnection, counter: &Arc<AtomicUsize>) -> Self {
        Self::TracedPgTx(conn, settings_for(counter))
    }

    pub(crate) fn traced_sqlite_tx(
        conn: &'a mut SqliteConnection,
        counter: &Arc<AtomicUsize>,
    ) -> Self {
        Self::TracedSqliteTx(conn, settings_for(counter))
    }

    fn settings(&self) -> Option<Arc<QuerySettings>> {
        match self {
            Self::TracedPgPool(_, settings)
            | Self::TracedPgTx(_, settings)
            | Self::TracedSqlitePool(_, settings)
            | Self::TracedSqliteTx(_, settings) => Some(settings.clone()),
            _ => None,
        }
    }
}

async fn observe_query<T, F, R>(
    sql: &str,
    settings: Option<Arc<QuerySettings>>,
    operation: F,
    row_count: R,
) -> Result<T, sqlx::Error>
where
    F: Future<Output = Result<T, sqlx::Error>>,
    R: FnOnce(&T) -> Option<u64>,
{
    let statement = redact_statement(sql);
    let model = query_target(sql);
    let span = tracing::info_span!(
        "db.query",
        db.system = "sqlx",
        db.statement = %statement,
        db.model = model.as_deref().unwrap_or("unknown"),
        db.rows = tracing::field::Empty,
        db.duration_ms = tracing::field::Empty,
        db.success = tracing::field::Empty,
    );
    let started = Instant::now();
    let result = operation.instrument(span.clone()).await;
    let duration = started.elapsed();
    let rows = match &result {
        Ok(value) => row_count(value),
        Err(_) => None,
    };
    span.record("db.rows", rows);
    span.record("db.duration_ms", duration.as_secs_f64() * 1000.0);
    span.record("db.success", result.is_ok());

    let threshold = settings
        .as_ref()
        .map(|settings| settings.slow_threshold_nanos.load(Ordering::Relaxed))
        .unwrap_or(u64::MAX);
    let slow = threshold != u64::MAX && duration.as_nanos() >= threshold as u128;
    let event = QueryEvent {
        statement,
        model,
        rows,
        duration,
        success: result.is_ok(),
        slow,
    };
    if let Some(settings) = settings {
        if slow {
            tracing::warn!(
                parent: &span,
                db.statement = %event.statement,
                db.model = event.model.as_deref().unwrap_or("unknown"),
                db.duration_ms = duration.as_secs_f64() * 1000.0,
                "slow database query"
            );
        }
        let observer = settings
            .observer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(observer) = observer {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| observer(&event)));
        }
    }
    result
}

fn redact_statement(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            output.push('?');
            while let Some(inner) = chars.next() {
                if inner == '\\' {
                    let _ = chars.next();
                } else if inner == '\'' {
                    if chars.peek() == Some(&'\'') {
                        let _ = chars.next();
                    } else {
                        break;
                    }
                }
            }
        } else {
            output.push(ch);
        }
    }
    output
}

fn query_target(sql: &str) -> Option<String> {
    let tokens = sql.split_whitespace().collect::<Vec<_>>();
    let upper = tokens
        .iter()
        .map(|token| {
            token
                .trim_matches(|c: char| c == '(' || c == ')')
                .to_ascii_uppercase()
        })
        .collect::<Vec<_>>();
    let target_index = match upper.first().map(String::as_str) {
        Some("SELECT") | Some("WITH") | Some("DELETE") => {
            upper.iter().position(|t| t == "FROM")? + 1
        }
        Some("INSERT") => upper.iter().position(|t| t == "INTO")? + 1,
        Some("UPDATE") => 1,
        _ => return None,
    };
    let target = tokens.get(target_index)?.trim_matches(|c: char| {
        c == '"' || c == '`' || c == '[' || c == ']' || c == '(' || c == ')'
    });
    let table = target.rsplit('.').next().unwrap_or(target);
    let name = table
        .trim_matches('"')
        .trim_matches('`')
        .trim_matches('[')
        .trim_matches(']');
    (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .then(|| name.to_owned())
}

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
        let settings = self.settings();
        observe_query(
            sql,
            settings,
            async {
                match self {
                    Conn::PgPool(pool) | Conn::TracedPgPool(pool, _) => {
                        let rows = build_pg_query(sql, params).fetch_all(*pool).await?;
                        Ok(rows.into_iter().map(DbRow::Pg).collect())
                    }
                    Conn::PgTx(tx) | Conn::TracedPgTx(tx, _) => {
                        let rows = build_pg_query(sql, params).fetch_all(&mut **tx).await?;
                        Ok(rows.into_iter().map(DbRow::Pg).collect())
                    }
                    Conn::SqlitePool(pool) | Conn::TracedSqlitePool(pool, _) => {
                        let rows = build_sqlite_query(sql, params).fetch_all(*pool).await?;
                        Ok(rows.into_iter().map(DbRow::Sqlite).collect())
                    }
                    Conn::SqliteTx(tx) | Conn::TracedSqliteTx(tx, _) => {
                        let rows = build_sqlite_query(sql, params).fetch_all(&mut **tx).await?;
                        Ok(rows.into_iter().map(DbRow::Sqlite).collect())
                    }
                }
            },
            |rows: &Vec<DbRow>| Some(rows.len() as u64),
        )
        .await
    }

    /// Execute the query, requiring exactly one row.
    pub async fn fetch_one(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<DbRow, sqlx::Error> {
        let settings = self.settings();
        observe_query(
            sql,
            settings,
            async {
                match self {
                    Conn::PgPool(pool) | Conn::TracedPgPool(pool, _) => Ok(DbRow::Pg(
                        build_pg_query(sql, params).fetch_one(*pool).await?,
                    )),
                    Conn::PgTx(tx) | Conn::TracedPgTx(tx, _) => Ok(DbRow::Pg(
                        build_pg_query(sql, params).fetch_one(&mut **tx).await?,
                    )),
                    Conn::SqlitePool(pool) | Conn::TracedSqlitePool(pool, _) => Ok(DbRow::Sqlite(
                        build_sqlite_query(sql, params).fetch_one(*pool).await?,
                    )),
                    Conn::SqliteTx(tx) | Conn::TracedSqliteTx(tx, _) => Ok(DbRow::Sqlite(
                        build_sqlite_query(sql, params).fetch_one(&mut **tx).await?,
                    )),
                }
            },
            |_| Some(1),
        )
        .await
    }

    /// Execute the query, returning the row if there is one.
    pub async fn fetch_optional(
        &mut self,
        sql: &str,
        params: &[BindValue],
    ) -> Result<Option<DbRow>, sqlx::Error> {
        let settings = self.settings();
        observe_query(
            sql,
            settings,
            async {
                match self {
                    Conn::PgPool(pool) | Conn::TracedPgPool(pool, _) => {
                        Ok(build_pg_query(sql, params)
                            .fetch_optional(*pool)
                            .await?
                            .map(DbRow::Pg))
                    }
                    Conn::PgTx(tx) | Conn::TracedPgTx(tx, _) => Ok(build_pg_query(sql, params)
                        .fetch_optional(&mut **tx)
                        .await?
                        .map(DbRow::Pg)),
                    Conn::SqlitePool(pool) | Conn::TracedSqlitePool(pool, _) => {
                        Ok(build_sqlite_query(sql, params)
                            .fetch_optional(*pool)
                            .await?
                            .map(DbRow::Sqlite))
                    }
                    Conn::SqliteTx(tx) | Conn::TracedSqliteTx(tx, _) => {
                        Ok(build_sqlite_query(sql, params)
                            .fetch_optional(&mut **tx)
                            .await?
                            .map(DbRow::Sqlite))
                    }
                }
            },
            |row: &Option<DbRow>| Some(u64::from(row.is_some())),
        )
        .await
    }

    /// Execute the query for its side effect, returning the affected-row count.
    pub async fn execute(&mut self, sql: &str, params: &[BindValue]) -> Result<u64, sqlx::Error> {
        let settings = self.settings();
        observe_query(
            sql,
            settings,
            async {
                match self {
                    Conn::PgPool(pool) | Conn::TracedPgPool(pool, _) => {
                        Ok(build_pg_query(sql, params)
                            .execute(*pool)
                            .await?
                            .rows_affected())
                    }
                    Conn::PgTx(tx) | Conn::TracedPgTx(tx, _) => Ok(build_pg_query(sql, params)
                        .execute(&mut **tx)
                        .await?
                        .rows_affected()),
                    Conn::SqlitePool(pool) | Conn::TracedSqlitePool(pool, _) => {
                        Ok(build_sqlite_query(sql, params)
                            .execute(*pool)
                            .await?
                            .rows_affected())
                    }
                    Conn::SqliteTx(tx) | Conn::TracedSqliteTx(tx, _) => {
                        Ok(build_sqlite_query(sql, params)
                            .execute(&mut **tx)
                            .await?
                            .rows_affected())
                    }
                }
            },
            |rows| Some(*rows),
        )
        .await
    }

    /// The database dialect for this connection handle.
    pub fn dialect(&self) -> Dialect {
        match self {
            Conn::PgPool(_) | Conn::PgTx(_) | Conn::TracedPgPool(_, _) | Conn::TracedPgTx(_, _) => {
                Dialect::Postgres
            }
            Conn::SqlitePool(_)
            | Conn::SqliteTx(_)
            | Conn::TracedSqlitePool(_, _)
            | Conn::TracedSqliteTx(_, _) => Dialect::Sqlite,
        }
    }

    /// Whether this handle is inside an open transaction.
    pub fn in_transaction(&self) -> bool {
        matches!(
            self,
            Conn::PgTx(_) | Conn::SqliteTx(_) | Conn::TracedPgTx(_, _) | Conn::TracedSqliteTx(_, _)
        )
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

impl DbExecutor for &mut PgConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::PgTx(self)
    }
}

impl DbExecutor for &mut SqliteConnection {
    fn conn(&mut self) -> Conn<'_> {
        Conn::SqliteTx(self)
    }
}

impl DbExecutor for Conn<'_> {
    fn conn(&mut self) -> Conn<'_> {
        match self {
            Conn::PgPool(pool) => Conn::PgPool(pool),
            Conn::PgTx(tx) => Conn::PgTx(tx),
            Conn::SqlitePool(pool) => Conn::SqlitePool(pool),
            Conn::SqliteTx(tx) => Conn::SqliteTx(tx),
            Conn::TracedPgPool(pool, settings) => Conn::TracedPgPool(pool, settings.clone()),
            Conn::TracedPgTx(tx, settings) => Conn::TracedPgTx(tx, settings.clone()),
            Conn::TracedSqlitePool(pool, settings) => {
                Conn::TracedSqlitePool(pool, settings.clone())
            }
            Conn::TracedSqliteTx(tx, settings) => Conn::TracedSqliteTx(tx, settings.clone()),
        }
    }
}

#[cfg(test)]
mod instrumentation_tests {
    use super::{query_target, redact_statement};

    #[test]
    fn redacts_quoted_values_and_infers_query_target() {
        let sql = "SELECT * FROM \"app_users\" WHERE email = 'person@example.test' AND note = 'it''s private'";
        let redacted = redact_statement(sql);
        assert!(!redacted.contains("person@example.test"));
        assert!(!redacted.contains("it's private"));
        assert!(redacted.contains("\"app_users\""));
        assert_eq!(query_target(sql).as_deref(), Some("app_users"));
        assert_eq!(
            query_target("UPDATE users SET name = ?").as_deref(),
            Some("users")
        );
        assert_eq!(
            query_target("INSERT INTO users (name) VALUES (?)").as_deref(),
            Some("users")
        );
    }
}
