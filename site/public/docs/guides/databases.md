# Databases and Backends

Djangors supports both PostgreSQL and SQLite backends. This dual-backend architecture allows you to run your application against a lightweight SQLite database for local development, testing, and single-writer deployments, while deploying to a highly concurrent PostgreSQL database in production.

---

## Selecting the Backend

The active database backend is determined dynamically from the `DATABASE_URL` environment variable at runtime. Djangors parses this URL to determine the appropriate `Dialect` without making an active connection.

In code, this is handled by `Dialect::from_url`. The configuration parses the URL according to the following rules:

* **SQLite backend**: Selected if the URL starts with `sqlite://`, ends with `.db` or `.sqlite`, or is exactly `:memory:` or `sqlite::memory:`.
* **PostgreSQL backend**: Selected for any other URL format (e.g. `postgres://` or `postgresql://`).

The connection string comes from the `DATABASE_URL` environment variable. Note that it is *not* part
of `djangors.toml` — [`DjangorsSettings`](settings.md) covers `debug`, `allowed_hosts`,
`secret_key`, `host`, and `port` only, and the database URL is read from the environment directly:

```bash
# Dev environment using SQLite
export DATABASE_URL="sqlite://db.sqlite3"

# An in-memory database, which is what the SQLite tests use
export DATABASE_URL="sqlite::memory:"

# Production using PostgreSQL
export DATABASE_URL="postgres://postgres:password@localhost:5432/my_app"
```

Pool sizing (`max_connections`, timeouts) is configured in code through
`djangors_db::config::DatabaseConfig`, not through a file:

```rust,illustrative
let config = djangors_db::config::DatabaseConfig::new(std::env::var("DATABASE_URL")?);
let db = djangors_db::Database::connect(&config).await?;
```

### Cargo Feature Flags (`postgres` and `sqlite`)

At compile time, `djangors-db` (and the `djangors` facade crate) gate each SQL driver behind its own Cargo feature flag (`postgres` and `sqlite`, both enabled by default). If your production deployment targets only one database engine, disable default features and enable only the backend you need to drop the unused driver (such as bundled `libsqlite3-sys` or `sqlx-postgres`) from your dependency graph and release binary:

```toml
# PostgreSQL-only build (drops sqlx-sqlite and libsqlite3-sys)
djangors = { version = "0.9", default-features = false, features = ["postgres", "rest"] }

# Or when depending on djangors-db / djangors-orm directly:
djangors-db = { version = "0.9", default-features = false, features = ["postgres"] }
djangors-orm = { version = "0.9", default-features = false, features = ["postgres"] }
```

> [!NOTE]
> Enabling neither `postgres` nor `sqlite` emits a clear `compile_error!` instructing you to enable at least one backend feature.

---

## Dialect-Aware Architecture

Rather than forcing a single SQL syntax across different databases, Djangors uses a dialect-aware architecture. The system checks the active `Dialect` to generate and execute database-specific SQL.

Dialect awareness spans across:
1. **ORM Query Path**: The generated SQL for filtering, limits, offsets, and operators is tailored to the active database dialect.
2. **Raw SQL Execution**: All production raw SQL built using the database executor uses the correct syntax and placeholder formats.
3. **Migrations**: When generating migration files via `dj makemigrations` and executing them via `dj migrate`, Djangors generates SQL operations tailored to Postgres or SQLite.

### The `Dialect` API

The `Dialect` enum resides in `djangors-db` and exposes the differences in SQL syntax:

```rust,illustrative
pub enum Dialect {
    Postgres,
    Sqlite,
}
```

It provides the following helper methods to bridge dialect differences:

* **`placeholder(index)`**: Returns `"$1"`, `"$2"`, etc., for PostgreSQL, and `"?"` for SQLite.
* **`ilike()`**: Returns `"ILIKE"` for PostgreSQL. For SQLite, it returns `"LIKE"` because SQLite's standard `LIKE` operator is already ASCII-case-insensitive.
* **`quote_ident(name)`**: Wraps the identifier in double quotes (e.g. `"name"`). Supported by both Postgres and SQLite.
* **`cast_float(expr)`**: Casts an expression to a float type: `expr::float8` on PostgreSQL, and `CAST(expr AS REAL)` on SQLite.
* **`extract_date_part(part, col)`**: Extracts parts of a date (`Year`, `Month`, `Day`). Generates `EXTRACT(part FROM col)::int` for Postgres and `CAST(strftime(format, col) AS INTEGER)` for SQLite.
* **`bytea_type()`**: Returns `"BYTEA"` for PostgreSQL, and `"BLOB"` for SQLite.
* **`auto_pk_type()`**: Returns `"SERIAL PRIMARY KEY"` for PostgreSQL, and `"INTEGER PRIMARY KEY AUTOINCREMENT"` for SQLite.
* **`timestamp_type()`**: Returns `"TIMESTAMPTZ"` for PostgreSQL, and `"TEXT"` for SQLite.
* **`current_timestamp()`**: Returns `"now()"` for PostgreSQL, and `"CURRENT_TIMESTAMP"` for SQLite.
* **`from_url(url)`**: Static helper that infers the dialect from a URL string.

---

## Honest Limitations

While the dual-backend architecture simplifies local development and CI testing, there are key differences and limitations to keep in mind:

### 1. Dual-Backend Testing vs. PostgreSQL-Specific Features
`djangors-test` (`TestDatabase` and `TestBackend`) supports **both** in-memory SQLite (`TEST_BACKEND=sqlite`) and PostgreSQL (`TEST_BACKEND=postgres`). In-memory SQLite gives fast, zero-container test runs for standard ORM, view, auth, admin, and REST tests. However, PostgreSQL-specific capabilities—such as session-level advisory locks (`acquire_cross_process_lock`), `SELECT ... FOR UPDATE SKIP LOCKED` / `NOWAIT`, PostgreSQL full-text search (`tsvector`), and PostgreSQL transaction isolation levels—require a real PostgreSQL instance and return `UnsupportedOnDialect` (or skip) on SQLite.

### 2. Unsupported Migrations on SQLite
SQLite has very limited support for altering existing table schemas. Because of this, certain schema operations are not supported on SQLite. For instance:
* `Operation::AlterColumnType` returns a `MigrationError::UnsupportedOnDialect` rather than emitting broken or corrupt SQL.

### 3. SQLite Concurrency (Single Writer)
SQLite permits a single writer at any given time. If your application has highly concurrent write traffic, SQLite will serialize these writes and may return database locked errors (`database is locked`) if writes timeout. Postgres handles highly concurrent writes out of the box using fine-grained row locks.

### 4. Database Handles and the Connection Pool
When interacting with the connection pool:
* Calling `Database::pool()` returns a reference to the underlying `PgPool`. **This will panic** if the database handle is connected to SQLite.
* `Database::sqlite_pool()` is the safe SQLite counterpart: it returns `Option<&SqlitePool>` — `Some` on a SQLite handle, `None` on a PostgreSQL handle.
* To write code that runs on both backends, use `Database::conn()` instead. It returns a `Conn` enum wrapper that can execute queries on both PostgreSQL and SQLite pools.

---

## When to Choose Which

* **SQLite** is ideal for:
  * Local development and fast `TEST_BACKEND=sqlite cargo test` runs without a local PostgreSQL container.
  * Simple staging environments, embedded tools, or internal apps with minimal write concurrency.
  * Single-writer, read-heavy workloads.
* **PostgreSQL** is required for:
  * Production deployments with concurrent writes or high traffic.
  * Testing PostgreSQL-specific locking (`SKIP LOCKED`, advisory locks), full-text search, or isolation levels.
  * Advanced schema operations such as `AlterColumnType` migrations.

---

## Row Locking and Savepoints

For banking and financial applications requiring atomic transactions, Djangors provides explicit row locking (`select_for_update`) and savepoints (`savepoint`, `rollback_to_savepoint`, `release_savepoint`).

### Row Locking with `select_for_update`

`QuerySet::select_for_update` locks matching rows for the duration of a transaction:

```rust,illustrative
Account::objects()
    .filter(q!(id = sender_id))?
    .select_for_update()
    .get(&mut *conn)
    .await?;
```

Modifiers allow controlling wait and table locking behavior:

* **`.nowait()`**: Returns an error immediately if a row lock cannot be acquired instead of blocking.
* **`.skip_locked()`**: Skips rows locked by concurrent transactions.
* **`.lock_of(&["account"])`**: Limits locking to specific tables in Postgres (`FOR UPDATE OF`).

> [!NOTE]
> `select_for_update()` requires an active transaction; calling it outside a transaction returns `OrmError::SelectForUpdateOutsideTransaction`. On SQLite, `select_for_update()` degrades gracefully to a no-op because SQLite serializes writes at the database level, but modifiers (`nowait`, `skip_locked`, `lock_of`) return `OrmError::UnsupportedOnDialect`.

### Savepoints

Savepoints allow partial rollback within a transaction handle (`Conn`). A savepoint only makes sense inside an open transaction: calling `savepoint` on a `Conn` obtained from the pool (`db.conn()`) returns `DbError::SavepointOutsideTransaction`, so issue them from inside `transaction_conn(...)` (or ORM-managed transactions) — supported on both backends:

```rust,illustrative
conn.savepoint("transfer_checkpoint").await?;
// Perform risky operation
if let Err(_) = process_fee(&mut conn).await {
    conn.rollback_to_savepoint("transfer_checkpoint").await?;
} else {
    conn.release_savepoint("transfer_checkpoint").await?;
}
```

### Double-Entry Ledger Example

Below is a banking transfer example that debits one account and credits another under row locking with savepoint error recovery:

```rust,illustrative
pub async fn transfer_funds(
    db: &Database,
    sender_id: i64,
    receiver_id: i64,
    amount: i64,
) -> Result<(), DbError> {
    db.transaction_conn(|conn| {
        Box::pin(async move {
            // Lock sender account
            let mut sender = Account::objects()
                .filter(q!(id = sender_id))
                .map_err(OrmError::from)?
                .select_for_update()
                .get(&mut **conn)
                .await?;

            if sender.balance < amount {
                return Err(DbError::TransactionFailed("Insufficient funds".into()));
            }

            // Lock receiver account
            let mut receiver = Account::objects()
                .filter(q!(id = receiver_id))
                .map_err(OrmError::from)?
                .select_for_update()
                .get(&mut **conn)
                .await?;

            // Create a savepoint before attempting ledger operations
            conn.savepoint("ledger_entry").await?;

            sender.balance -= amount;
            receiver.balance += amount;

            // Execute debit & credit updates inside the transaction handle
            conn.execute(
                "UPDATE accounts SET balance = $1 WHERE id = $2",
                &[BindValue::I64(sender.balance), BindValue::I64(sender.id)],
            )
            .await?;
            conn.execute(
                "UPDATE accounts SET balance = $1 WHERE id = $2",
                &[BindValue::I64(receiver.balance), BindValue::I64(receiver.id)],
            )
            .await?;

            // Commit savepoint checkpoint
            conn.release_savepoint("ledger_entry").await?;

            Ok(())
        })
    })
    .await
}

---

## Executor, Rows, and Raw SQL

Beyond the ORM, `djangors-db` exposes a small executor layer for running raw SQL that works identically on PostgreSQL and SQLite. Everything hangs off a single borrowed handle — `Conn` — which can point at either a connection pool or an open transaction.

### The `Conn` Executor

`djangors_db::executor::Conn` is the low-level query runner. You obtain one for a pool with `db.conn()` — compare that to `Database::pool()`, which returns a `&PgPool` and is PostgreSQL-only.

`Conn` is an enum over four targets:

* `Conn::PgPool(&PgPool)` — a query against the PostgreSQL pool.
* `Conn::PgTx(&mut PgConnection)` — a query inside a PostgreSQL transaction.
* `Conn::SqlitePool(&SqlitePool)` — a query against the SQLite pool.
* `Conn::SqliteTx(&mut SqliteConnection)` — a query inside a SQLite transaction.

That variety is why the same call sites run on either backend: the pool variants back `db.conn()`, while `db.transaction_conn(...)` hands its closure a transaction variant.

The executor methods all take a raw SQL string plus a slice of `BindValue` parameters:

* `fetch_all(&mut self, sql, &[BindValue]) -> Result<Vec<DbRow>, sqlx::Error>` — run the query and collect every row.
* `fetch_one(&mut self, sql, &[BindValue]) -> Result<DbRow, sqlx::Error>` — require exactly one row.
* `fetch_optional(&mut self, sql, &[BindValue]) -> Result<Option<DbRow>, sqlx::Error>` — zero or one row.
* `execute(&mut self, sql, &[BindValue]) -> Result<u64, sqlx::Error>` — run a statement and return the affected-row count.
* `dialect(&self) -> Dialect` — which backend this handle targets.
* `in_transaction(&self) -> bool` — whether this handle is a transaction variant.
* `savepoint` / `rollback_to_savepoint` / `release_savepoint` — see [Savepoints](#savepoints).

> [!NOTE]
> For dual-backend code written once, reach for `Conn` (via `db.conn()`) rather than `Database::pool()`. `pool()` returns a `&PgPool` and panics on a SQLite handle; `conn()` returns a `Conn` and runs on both. `Conn` is also the handle shape the savepoint helpers and `transaction_conn` use, so executor code ports directly into a transaction.

Here is `fetch_one` with a bound parameter. Postgres uses `$1` placeholders and SQLite uses `?`, but the `Conn` API is identical — only the SQL text changes:

```rust,compile
# use djangors_db::BindValue;
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
# let id: i64 = 42;
let row = db.conn().fetch_one("SELECT name FROM accounts WHERE id = $1", &[BindValue::I64(id)]).await?;
# Ok(())
# }
```

### Bind Parameters

`BindValue` (in `djangors_db::bind`) is a typed, backend-agnostic query parameter:

* `I64(i64)` — 64-bit integer.
* `F64(f64)` — 64-bit float.
* `Text(String)` — text.
* `Bool(bool)` — boolean.
* `DateTime(chrono::DateTime<chrono::Utc>)` — UTC timestamp.
* `Bytes(Vec<u8>)` — byte vector.
* `Null(NullKind)` — a typed SQL `NULL`.

`NullKind` selects the SQL type a `NULL` is bound as: `I64`, `F64`, `Text`, `Bool`, `DateTime`, or `Bytes`. The kind is mandatory — Postgres rejects a `NULL` parameter whose type cannot be inferred.

Constructing a parameter list:

```rust,compile
# use djangors_db::bind::{BindValue, NullKind};
# use chrono::Utc;
# fn build_params() {
let params = vec![
    BindValue::Text("alice".to_string()),
    BindValue::I64(42),
    BindValue::F64(1.5),
    BindValue::Bool(true),
    BindValue::DateTime(Utc::now()),
    BindValue::Bytes(vec![0, 1, 2]),
    BindValue::Null(NullKind::DateTime),
];
# let _ = params;
# }
```

### Reading Rows

The `fetch_*` methods return `DbRow`, a wrapper over the backend row (`PgRow` or `SqliteRow`). Every accessor returns `Result<Option<T>, sqlx::Error>` — `None` for a SQL `NULL`, an error when the column cannot be decoded. The integer accessors additionally accept narrower `i32`/`i16` columns.

By column index:

* `try_i64(idx)`, `try_f64(idx)`, `try_string(idx)`, `try_bool(idx)`, `try_datetime(idx)`, `try_bytes(idx)`.

By column name:

* `try_i64_by_name(name)`, `try_f64_by_name(name)`, `try_string_by_name(name)`, `try_bool_by_name(name)`, `try_datetime_by_name(name)`, `try_bytes_by_name(name)`.

Reading a column by name needs no extra imports — it is an inherent method on `DbRow`:

```rust,compile
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let row = db.conn().fetch_one("SELECT 1 AS one", &[]).await?;
let v: Option<i64> = row.try_i64_by_name("one")?;
assert_eq!(v, Some(1));
# Ok(())
# }
```

The `_by_name` variants are what the ORM relies on: the `FromRow` impl generated by `#[derive(Model)]` decodes every field through `row.try_*_by_name(column)` (see `djangors-macros`), never by position. Positional decoding is confined to this raw executor layer, which is why `q!`-based `eq` filters keep resolving correctly across renamed columns (`#[field(column = "...")]`): filters and `FromRow` both key off the resolved column name, so the two stay in lockstep.

### Transactions and Isolation Levels

`Database::transaction(f)` and `Database::transaction_with_isolation(level, f)` run `f` with a `&mut PgConnection` and are **PostgreSQL-only** — on a SQLite handle they return `DbError::TransactionFailed`. `Database::transaction_conn(f)` passes a `&mut Conn` instead and works on **both** backends; it is also the transaction entry point the `Conn` savepoint helpers expect.

Isolation levels, from `djangors_db::IsolationLevel`:

* `ReadCommitted` — the PostgreSQL default.
* `RepeatableRead`
* `Serializable`

`transaction_with_isolation(level, f)` begins the transaction and issues the matching `SET TRANSACTION ISOLATION LEVEL ...` before running `f`.
```
