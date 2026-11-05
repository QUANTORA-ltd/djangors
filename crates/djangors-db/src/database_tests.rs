use super::*;

const TEST_DB_URL: &str = "postgres://postgres:postgres@localhost/djangors_test";

#[test]
fn test_config_builder() {
    let config = DatabaseConfig::new(TEST_DB_URL)
        .max_connections(5)
        .min_connections(2)
        .connect_timeout_secs(15)
        .idle_timeout_secs(Some(30));

    assert_eq!(config.url, TEST_DB_URL);
    assert_eq!(config.max_connections, 5);
    assert_eq!(config.min_connections, 2);
    assert_eq!(config.connect_timeout_secs, 15);
    assert_eq!(config.idle_timeout_secs, Some(30));
}

#[test]
fn test_isolation_level_mapping() {
    assert_eq!(
        isolation_level_sql(IsolationLevel::ReadCommitted),
        "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"
    );
    assert_eq!(
        isolation_level_sql(IsolationLevel::RepeatableRead),
        "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"
    );
    assert_eq!(
        isolation_level_sql(IsolationLevel::Serializable),
        "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn test_database_connect_and_query() {
    // Requires Postgres server and DATABASE_URL.
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = DatabaseConfig::new(&url);
    let db = Database::connect(&config).await.expect("Failed to connect");

    let row: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(db.pool())
        .await
        .expect("Failed query");
    assert_eq!(row.0, 1);
}

#[tokio::test]
async fn test_sqlite_connect_in_memory() {
    let config = DatabaseConfig::new(":memory:");
    let db = Database::connect(&config)
        .await
        .expect("Failed to connect SQLite");
    assert_eq!(db.dialect(), Dialect::Sqlite);
    assert!(db.sqlite_pool().is_some());
}

#[derive(Debug)]
#[allow(dead_code)]
enum TypedTransactionError {
    Business(&'static str),
    Database(DbError),
}

impl From<DbError> for TypedTransactionError {
    fn from(error: DbError) -> Self {
        Self::Database(error)
    }
}

#[tokio::test]
async fn transaction_conn_preserves_typed_error_and_rolls_back() {
    let db = Database::connect(&DatabaseConfig::new("sqlite::memory:").max_connections(1))
        .await
        .expect("connect sqlite");
    db.conn()
        .execute("CREATE TABLE typed_tx_test (value TEXT)", &[])
        .await
        .unwrap();

    let result: Result<(), TypedTransactionError> = db
        .transaction_conn(|conn| {
            Box::pin(async move {
                conn.execute(
                    "INSERT INTO typed_tx_test (value) VALUES ('transient')",
                    &[],
                )
                .await
                .map_err(DbError::from)?;
                Err(TypedTransactionError::Business("no copies left"))
            })
        })
        .await;

    assert!(matches!(
        result,
        Err(TypedTransactionError::Business("no copies left"))
    ));
    let row = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM typed_tx_test", &[])
        .await
        .unwrap();
    assert_eq!(row.try_i64(0).unwrap(), Some(0));
}

// Restored during review of dispatch 13.1: these three transaction tests were
// deleted by the SQLite port even though `transaction` and
// `transaction_with_isolation` both still exist. They cover the exact code the
// port rewrote, so losing them would have removed coverage precisely where the
// risk was highest. The rollback test's regression note is retained verbatim.

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn test_transaction_commit() {
    // Requires Postgres PgConnection transaction features.
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = DatabaseConfig::new(&url);
    let db = Database::connect(&config).await.expect("Failed to connect");

    let val = db
        .transaction(|conn| {
            Box::pin(async move {
                sqlx::query("CREATE TEMPORARY TABLE test_tx_commit (id INT)")
                    .execute(&mut *conn)
                    .await
                    .map_err(DbError::QueryFailed)?;

                sqlx::query("INSERT INTO test_tx_commit VALUES (42)")
                    .execute(&mut *conn)
                    .await
                    .map_err(DbError::QueryFailed)?;

                let row: (i32,) = sqlx::query_as("SELECT id FROM test_tx_commit")
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(DbError::QueryFailed)?;

                Ok::<i32, DbError>(row.0)
            })
        })
        .await
        .expect("Transaction failed");

    assert_eq!(val, 42);
}

/// Regression note: this test previously used a `TEMPORARY TABLE`, created on a
/// separately-acquired connection and then written to (and checked) from a
/// second, distinct pooled connection inside `db.transaction(...)`. Postgres
/// temp tables are strictly session-scoped, so the transaction's `INSERT` was
/// actually failing with "relation does not exist" (SQLSTATE 42P01) — not the
/// intentional `Err` this test meant to trigger — meaning the test passed
/// regardless of whether `Database::transaction` ever called `.rollback()` at
/// all. Confirmed empirically by instrumenting the error and inspecting it
/// directly. Fixed by using a real (non-temporary) table, which — unlike a
/// temp table — is visible from every connection in the pool, so the INSERT
/// genuinely lands inside the transaction and the subsequent rollback is what
/// the final count actually proves.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn test_transaction_rollback() {
    // Requires Postgres PgConnection transaction features.
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = DatabaseConfig::new(&url);
    let db = Database::connect(&config).await.expect("Failed to connect");

    sqlx::query("DROP TABLE IF EXISTS test_tx_rollback")
        .execute(db.pool())
        .await
        .expect("Failed to drop pre-existing table");
    sqlx::query("CREATE TABLE test_tx_rollback (id INT)")
        .execute(db.pool())
        .await
        .expect("Failed to create table");

    let tx_res = db
        .transaction(|conn| {
            Box::pin(async move {
                sqlx::query("INSERT INTO test_tx_rollback VALUES (100)")
                    .execute(&mut *conn)
                    .await
                    .map_err(DbError::QueryFailed)?;

                Err::<(), DbError>(DbError::TransactionFailed(
                    "Intentional rollback".to_string(),
                ))
            })
        })
        .await;

    assert!(tx_res.is_err());
    assert!(
        matches!(&tx_res, Err(DbError::TransactionFailed(msg)) if msg == "Intentional rollback"),
        "expected the intentional error to propagate unchanged, got: {tx_res:?}"
    );

    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM test_tx_rollback")
        .fetch_one(db.pool())
        .await
        .expect("Failed to query table");

    assert_eq!(
        row.0, 0,
        "row must not be visible — the transaction should have rolled back"
    );

    sqlx::query("DROP TABLE test_tx_rollback")
        .execute(db.pool())
        .await
        .expect("Failed to clean up table");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn test_transaction_isolation_levels() {
    // Requires Postgres-specific transaction isolation level features (SHOW transaction_isolation).
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let config = DatabaseConfig::new(&url);
    let db = Database::connect(&config).await.expect("Failed to connect");

    for level in &[
        IsolationLevel::ReadCommitted,
        IsolationLevel::RepeatableRead,
        IsolationLevel::Serializable,
    ] {
        let actual_level = db
            .transaction_with_isolation(*level, |conn| {
                Box::pin(async move {
                    let row: (String,) = sqlx::query_as("SHOW transaction_isolation")
                        .fetch_one(&mut *conn)
                        .await
                        .map_err(DbError::QueryFailed)?;
                    Ok::<String, DbError>(row.0)
                })
            })
            .await
            .expect("Transaction with isolation failed");

        let expected = match level {
            IsolationLevel::ReadCommitted => "read committed",
            IsolationLevel::RepeatableRead => "repeatable read",
            IsolationLevel::Serializable => "serializable",
        };
        assert_eq!(actual_level.to_lowercase(), expected);
    }
}

#[tokio::test]
async fn test_transaction_conn_commit_and_rollback_both_backends() {
    // 1. Postgres backend (if DATABASE_URL is set)
    if let Ok(pg_url) = std::env::var("DATABASE_URL") {
        let pg_config = DatabaseConfig::new(&pg_url);
        let pg_db = Database::connect(&pg_config).await.expect("connect pg");

        sqlx::query("DROP TABLE IF EXISTS test_tx_conn_pg")
            .execute(pg_db.pool())
            .await
            .ok();
        sqlx::query("CREATE TABLE test_tx_conn_pg (id INT)")
            .execute(pg_db.pool())
            .await
            .expect("create pg table");

        let res: Result<i32, DbError> = pg_db
            .transaction_conn(|conn| {
                Box::pin(async move {
                    conn.execute("INSERT INTO test_tx_conn_pg VALUES (10)", &[])
                        .await
                        .map_err(DbError::QueryFailed)?;
                    Ok::<i32, DbError>(10)
                })
            })
            .await;
        assert_eq!(res.unwrap(), 10);

        let err_res: Result<(), DbError> = pg_db
            .transaction_conn(|conn| {
                Box::pin(async move {
                    conn.execute("INSERT INTO test_tx_conn_pg VALUES (20)", &[])
                        .await
                        .map_err(DbError::QueryFailed)?;
                    Err::<(), DbError>(DbError::TransactionFailed("abort".to_string()))
                })
            })
            .await;
        assert!(err_res.is_err());

        let row = pg_db
            .conn()
            .fetch_one("SELECT COUNT(*) FROM test_tx_conn_pg", &[])
            .await
            .unwrap();
        assert_eq!(row.try_i64(0).unwrap().unwrap(), 1);

        sqlx::query("DROP TABLE test_tx_conn_pg")
            .execute(pg_db.pool())
            .await
            .ok();
    }

    // 2. SQLite backend
    let sqlite_config = DatabaseConfig::new(":memory:");
    let sqlite_db = Database::connect(&sqlite_config)
        .await
        .expect("connect sqlite");

    sqlite_db
        .conn()
        .execute("CREATE TABLE test_tx_conn_sqlite (id INT)", &[])
        .await
        .expect("create sqlite table");

    let res: Result<i32, DbError> = sqlite_db
        .transaction_conn(|conn| {
            Box::pin(async move {
                conn.execute("INSERT INTO test_tx_conn_sqlite VALUES (100)", &[])
                    .await
                    .map_err(DbError::QueryFailed)?;
                Ok::<i32, DbError>(100)
            })
        })
        .await;
    assert_eq!(res.unwrap(), 100);

    let err_res: Result<(), DbError> = sqlite_db
        .transaction_conn(|conn| {
            Box::pin(async move {
                conn.execute("INSERT INTO test_tx_conn_sqlite VALUES (200)", &[])
                    .await
                    .map_err(DbError::QueryFailed)?;
                Err::<(), DbError>(DbError::TransactionFailed("abort".to_string()))
            })
        })
        .await;
    assert!(err_res.is_err());

    let row = sqlite_db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM test_tx_conn_sqlite", &[])
        .await
        .unwrap();
    assert_eq!(row.try_i64(0).unwrap().unwrap(), 1);
}

#[tokio::test]
async fn test_savepoint_name_validation() {
    let sqlite_config = DatabaseConfig::new(":memory:");
    let sqlite_db = Database::connect(&sqlite_config)
        .await
        .expect("connect sqlite");

    sqlite_db
        .transaction_conn(|conn| {
            Box::pin(async move {
                // Invalid names:
                assert!(matches!(
                    conn.savepoint("sp\"; DROP TABLE users; --").await,
                    Err(DbError::InvalidSavepointName(_))
                ));
                assert!(matches!(
                    conn.savepoint("1sp").await,
                    Err(DbError::InvalidSavepointName(_))
                ));
                assert!(matches!(
                    conn.savepoint("").await,
                    Err(DbError::InvalidSavepointName(_))
                ));
                let long_name = "a".repeat(64);
                assert!(matches!(
                    conn.savepoint(&long_name).await,
                    Err(DbError::InvalidSavepointName(_))
                ));

                // Valid names:
                assert!(conn.savepoint("sp1").await.is_ok());
                assert!(conn.savepoint("_sp2").await.is_ok());
                assert!(conn.release_savepoint("_sp2").await.is_ok());
                assert!(conn.rollback_to_savepoint("sp1").await.is_ok());

                Ok::<(), DbError>(())
            })
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_savepoint_outside_transaction() {
    let sqlite_config = DatabaseConfig::new(":memory:");
    let sqlite_db = Database::connect(&sqlite_config)
        .await
        .expect("connect sqlite");

    let mut conn = sqlite_db.conn();
    assert!(matches!(
        conn.savepoint("sp1").await,
        Err(DbError::SavepointOutsideTransaction)
    ));
    assert!(matches!(
        conn.rollback_to_savepoint("sp1").await,
        Err(DbError::SavepointOutsideTransaction)
    ));
    assert!(matches!(
        conn.release_savepoint("sp1").await,
        Err(DbError::SavepointOutsideTransaction)
    ));
}

#[tokio::test]
async fn test_savepoint_roundtrip_both_backends() {
    // 1. SQLite backend
    let sqlite_config = DatabaseConfig::new(":memory:");
    let sqlite_db = Database::connect(&sqlite_config)
        .await
        .expect("connect sqlite");

    sqlite_db
        .conn()
        .execute("CREATE TABLE test_sp_sqlite (id INT, val TEXT)", &[])
        .await
        .expect("create table sqlite");

    sqlite_db
        .transaction_conn(|conn| {
            Box::pin(async move {
                conn.execute("INSERT INTO test_sp_sqlite VALUES (1, 'first')", &[])
                    .await
                    .map_err(DbError::QueryFailed)?;

                conn.savepoint("sp1").await?;

                conn.execute("INSERT INTO test_sp_sqlite VALUES (2, 'second')", &[])
                    .await
                    .map_err(DbError::QueryFailed)?;

                conn.rollback_to_savepoint("sp1").await?;

                Ok::<(), DbError>(())
            })
        })
        .await
        .expect("savepoint tx sqlite");

    let count_row = sqlite_db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM test_sp_sqlite", &[])
        .await
        .unwrap();
    assert_eq!(count_row.try_i64(0).unwrap().unwrap(), 1);

    let row = sqlite_db
        .conn()
        .fetch_one("SELECT val FROM test_sp_sqlite WHERE id = 1", &[])
        .await
        .unwrap();
    assert_eq!(row.try_string(0).unwrap().unwrap(), "first");

    // 2. Postgres backend (if DATABASE_URL is set)
    if let Ok(pg_url) = std::env::var("DATABASE_URL") {
        let pg_config = DatabaseConfig::new(&pg_url);
        let pg_db = Database::connect(&pg_config).await.expect("connect pg");

        sqlx::query("DROP TABLE IF EXISTS test_sp_pg")
            .execute(pg_db.pool())
            .await
            .ok();
        sqlx::query("CREATE TABLE test_sp_pg (id INT, val TEXT)")
            .execute(pg_db.pool())
            .await
            .expect("create table pg");

        pg_db
            .transaction_conn(|conn| {
                Box::pin(async move {
                    conn.execute("INSERT INTO test_sp_pg VALUES (1, 'first')", &[])
                        .await
                        .map_err(DbError::QueryFailed)?;

                    conn.savepoint("sp1").await?;

                    conn.execute("INSERT INTO test_sp_pg VALUES (2, 'second')", &[])
                        .await
                        .map_err(DbError::QueryFailed)?;

                    conn.rollback_to_savepoint("sp1").await?;

                    Ok::<(), DbError>(())
                })
            })
            .await
            .expect("savepoint tx pg");

        let count_row = pg_db
            .conn()
            .fetch_one("SELECT COUNT(*) FROM test_sp_pg", &[])
            .await
            .unwrap();
        assert_eq!(count_row.try_i64(0).unwrap().unwrap(), 1);

        let row = pg_db
            .conn()
            .fetch_one("SELECT val FROM test_sp_pg WHERE id = 1", &[])
            .await
            .unwrap();
        assert_eq!(row.try_string(0).unwrap().unwrap(), "first");

        sqlx::query("DROP TABLE test_sp_pg")
            .execute(pg_db.pool())
            .await
            .ok();
    }
}
