# Testing

Djangors supports two testing paradigms: **in-process route testing** via `djangors-test` (`TestClient`) and **real-socket integration testing** over localhost TCP sockets.

---

## 1. In-Process Testing (`TestClient` & `TestDatabase`)

`djangors-test` provides an in-process client that executes requests directly through `Router::handle()` without binding network sockets.

### In-Process Route Tests
```rust,compile
use djangors_test::TestClient;
use djangors_core::{Router, Response, StatusCode, Request, PathParams, DjangorsError};
use djangors_sessions::Session;

async fn hello_handler(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "hello from test"))
}

#[tokio::test]
async fn test_hello_route() {
    let router = Router::new().get("/hello", hello_handler);
    let client = TestClient::new(router);

    client.get("/hello")
        .send()
        .await
        .assert_status(StatusCode::OK)
        .assert_contains("hello from test");
}
```

### `RequestBuilder::with_state` & reading `TestResponse`

Builders carry app state (`AppState`) into the request, and the resulting `TestResponse` exposes
the raw status and body for assertions beyond the `assert_*` helpers:

| Method | Signature | Notes |
| --- | --- | --- |
| `RequestBuilder::with_state<T>(state)` | `fn(self, T) -> Self` | Attach an `AppState` value; request handlers see it via `req.state::<T>()` |
| `TestResponse::body_str()` | `fn(&self) -> String` | The response body as lossy UTF-8 |
| `TestResponse::status()` | `fn(&self) -> StatusCode` | The raw HTTP status code |

```rust,compile
# use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode, Router};
# use djangors_test::TestClient;
#[derive(Clone, Debug)]
struct Db(u32);

async fn hi_handler(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req.state::<Db>().expect("state attached");
    Ok(Response::text(StatusCode::OK, &format!("db={}", db.0)))
}

# #[tokio::main]
# async fn main() {
let client = TestClient::new(Router::new().get("/hi", hi_handler));

let response = client.get("/hi").with_state(Db(7)).send().await;
assert_eq!(response.status(), StatusCode::OK);
assert_eq!(response.body_str(), "db=7");
# }
```

### Form & Session Testing
```rust,compile
# async fn submit_handler(_: djangors_core::Request, _: djangors_core::PathParams) -> Result<djangors_core::Response, djangors_core::DjangorsError> { Ok(djangors_core::Response::text(djangors_core::StatusCode::CREATED, "")) }
# use djangors_sessions::Session;
# use djangors_test::TestClient;
# use djangors_core::{Router, StatusCode};
#[tokio::test]
async fn test_authenticated_form_submit() {
    let router = Router::new().post("/submit", submit_handler);
    let client = TestClient::new(router);

    let session = Session::new_empty();
    session.set("user_id", 42i64);

    client.post_form("/submit", &[("title", "Test Item")])
        .with_session(session)
        .send()
        .await
        .assert_status(StatusCode::CREATED);
}
```

### Database Fixture (`TestDatabase`)
```rust,compile
use djangors_test::TestDatabase;

# #[ignore]
#[tokio::test]
async fn test_database_queries() {
    let test_db = TestDatabase::connect().await.unwrap();
    let db = test_db.database();

    test_db.create_table("CREATE TABLE test_table (id BIGSERIAL PRIMARY KEY)").await.unwrap();
    // ... run queries against db ...
    test_db.drop_table("test_table").await.unwrap();
}
```

#### Constructing a `TestDatabase`

`TestDatabase` offers several constructors covering the shared vs. isolated database models:

| Method | Signature | Returns |
| --- | --- | --- |
| `connect()` | `async fn() -> Result<Self, DbError>` | Backend chosen by `TEST_BACKEND`/`DATABASE_URL` (shared, no isolation) |
| `connect_url(url)` | `async fn(&str) -> Result<Self, DbError>` | Connect to an explicit URL (shared) |
| `new_for_backend(backend)` | `async fn(TestBackend) -> Result<Self, DbError>` | Fresh in-memory SQLite or isolated Postgres for that backend |
| `isolated()` | `async fn() -> Result<Self, DbError>` | Throwaway database (uniquely named Postgres, or in-memory SQLite) |
| `isolated_url(url)` | `async fn(&str) -> Result<Self, DbError>` | Same as `isolated()`, but using `url` as the base connection |

```rust,illustrative
use djangors_test::{TestBackend, TestDatabase};

// Non-isolated: run against an existing database (you manage the schema).
let shared = TestDatabase::connect_url("postgres://user:pass@localhost/my_app").await?;

// Isolated: create a uniquely-named throwaway DB, then drop it on cleanup().
let db = TestDatabase::isolated().await?;
// ... create tables, insert fixtures, run queries ...
db.cleanup().await?;

// Explicitly pick a backend regardless of environment.
let sqlite = TestDatabase::new_for_backend(TestBackend::Sqlite).await?;
```

##### `reset`, `cleanup`, `load_fixtures`

| Method | Signature | Notes |
| --- | --- | --- |
| `reset(&[tables])` | `async fn(&self, &[&str]) -> Result<(), sqlx::Error>` | `DROP TABLE IF EXISTS` each table in order |
| `cleanup()` | `async fn(self) -> Result<(), TestError>` | Drops the throwaway database (primary cleanup for isolated DBs) |
| `load_fixtures::<M>(json)` | `async fn(&self, &str) -> Result<Vec<M>, TestError>` | Deserialize JSON as `Vec<M>` and `bulk_create`; `M: Model + FromRow + DeserializeOwned` |

```rust,illustrative
use djangors_test::TestDatabase;

async fn seed(db: &TestDatabase) {
    // Load rows into a freshly-created table from a JSON array.
    let items = db.load_fixtures::<MyModel>(r#"[
        {"id": 1, "name": "Alice"},
        {"id": 2, "name": "Bob"}
    ]"#).await.unwrap();

    // Clear tables between tests.
    db.reset(&["my_model"]).await.unwrap();
}
```

SQLite in-memory databases need no `cleanup()` — they vanish with the connection (see
[Choosing a Database Backend](#choosing-a-database-backend)).

#### `TestBackend`

```rust,compile
# fn main() {
use djangors_test::TestBackend;

let current: TestBackend = TestBackend::current();

// TEST_BACKEND=sqlite           => Sqlite
// TEST_BACKEND=postgres         => Postgres
// unset: DATABASE_URL set       => Postgres
// unset: no DATABASE_URL        => Sqlite
match current {
    TestBackend::Postgres => println!("postgres"),
    TestBackend::Sqlite => println!("sqlite"),
}
# }
```

#### Cross-process advisory locks

`acquire_cross_process_lock` coordinates across *separate OS processes* (e.g. two crates racing to
`DROP`/`CREATE` the same table under `cargo test --workspace`) using a Postgres session-level
advisory lock. Unlike `std::sync::Mutex`, it spans test binaries:

```rust,illustrative
use djangors_test::{acquire_cross_process_lock, TestDatabase};

async fn exclusive_setup() {
    let db = TestDatabase::isolated().await.unwrap();

    let lock = acquire_cross_process_lock(db.database(), "shared-table-setup").await.unwrap();
    // ... create/drop the shared table ...
    lock.release().await.unwrap(); // explicit release frees the advisory lock promptly
```

| Method | Signature | Notes |
| --- | --- | --- |
| `acquire_cross_process_lock(db, name)` | `async fn(&Database, &str) -> Result<CrossProcessLock, sqlx::Error>` | Blocks until the named advisory lock is free |
| `CrossProcessLock::release()` | `async fn(self) -> Result<(), sqlx::Error>` | Releases the lock (`pg_advisory_unlock`) |

Release is explicit because Rust can't run async code in `Drop`; an un-released lock stays held
until its dedicated connection closes (i.e. when the test process exits).

### Model Factories (`#[derive(Factory)]`)

Writing out every field of a model for each test gets tedious. Derive `Factory` next to
`Model` and you get `Model::factory()`, a builder in the style of Python's `factory_boy`
or Laravel's model factories. It fills every field with a sensible default, and you set
only the fields the test cares about:

```rust,compile
# use djangors_orm::Model as _;
use djangors_db::Database;
use djangors_macros::{Factory, Model};

#[derive(Debug, Clone, Model, Factory)]
#[djangors(app = "blog")]
pub struct Article {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 120)]
    pub title: String,
    pub author_email: String,
    #[djangors(default = 10)]
    pub views: i64,
    pub subtitle: Option<String>,
}

async fn example(db: &Database) -> Result<(), djangors_orm::OrmError> {
    // In memory only, nothing saved: title "title_<n>", author_email "user<n>@example.com", ...
    let draft = Article::factory().build();
    assert_eq!(draft.views, 10); // `default = ...` attributes are respected
    assert_eq!(draft.subtitle, None); // Option fields default to None

    // Override just what matters, then save it.
    let popular = Article::factory().with_views(5_000).create(db).await?;
    assert!(popular.id > 0);

    // Save several at once; each one gets a fresh sequence number.
    let batch = Article::factory().with_title("Same title").create_many(db, 3).await?;
    assert_eq!(batch.len(), 3);
    Ok(())
}
```

| Field type | Default |
| --- | --- |
| `String` | `"<field>_<n>"`, cut to `max_length`; names containing `email` get `"user<n>@example.com"`, and names containing `url` or `website` get `"https://example.com/<field>/<n>"` |
| Integers and floats | The sequence number `n` |
| `bool` | `true` |
| `DateTime` | The current time |
| `Uuid` / `Decimal` | Built from `n` |
| `Option<T>` | `None` |
| Field with `choices` or `default = ...` | The first choice, or the default |
| Anything else | `Default::default()` |
| Auto primary key | `0` (the database assigns the real ID on `create`) |
| `ForeignKey<T>` / `OneToOne<T>` | ID `1` |

The sequence number `n` is shared by all factories of a model and increases with every
`build`, so unique columns don't collide.

`build` still uses ID `1` for relation fields. `create` and `create_many` require each
`ForeignKey` or `OneToOne` to be set explicitly; otherwise they return an `InvalidQuery`
that names the missing `.with_<field>_id(..)` call. Create the parent first and pass its ID:

```rust,illustrative
let author = Author::factory().create(&db).await?;
let book = Book::factory().with_author_id(author.id).create(&db).await?;
```

`Factory` is also re-exported as `djangors_test::Factory`.

---

## 2. Real-Socket Integration Testing

For full-stack integration testing (including Tower middleware layers, real HTTP headers, cookie handling, and graceful shutdown), test suites bind a local TCP socket on port `0`.

### Real-Socket Pattern (As Used in `examples/polls/tests/voting.rs`)
```rust,compile
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceBuilder;
use djangors_core::{Djangors, DjangorsSettings, Router};
use djangors_core::router::RouterService;
use djangors_core::middleware::{csrf_layer, security_headers_layer};
use djangors_sessions::{SessionLayer, SignedCookieStore};

# #[ignore]
#[tokio::test]
async fn test_full_stack_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let settings = DjangorsSettings::new()
        .with_host("127.0.0.1")
        .with_port(addr.port())
        .with_debug(true);

    let router = Router::new();
    let router_service = RouterService::new(router, settings.debug);

    let service = ServiceBuilder::new()
        .layer(security_headers_layer())
        .layer(SessionLayer::new(SignedCookieStore::new(b"test-secret-key-32-bytes-minimum!")))
        .layer(csrf_layer())
        .service(router_service);

    let app = Djangors::new(settings, Router::new());
    tokio::spawn(async move {
        app.serve_service(listener, service).await.unwrap();
    });

    // Send HTTP requests over real TCP connection
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.unwrap();

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let response = String::from_utf8_lossy(&buf);

    assert!(response.contains("200 OK"));
}
```

---

## Comparison Summary

| Metric | In-Process (`TestClient`) | Real-Socket Integration |
|---|---|---|
| **Speed** | Extremely fast (microseconds) | Fast (milliseconds) |
| **Network Overhead** | Zero (in-memory execution) | Real local TCP loopback socket |
| **Middleware Coverage** | Handler-level extensions & state | Full Tower layer stack & HTTP protocol |
| **Use Case** | Unit tests & route handler assertions | Full-stack end-to-end flow validation |

---

## Choosing a Database Backend

The test suite runs against either PostgreSQL or in-memory SQLite. Which one is used is decided by
`TEST_BACKEND`, falling back to whether `DATABASE_URL` is set:

| Condition | Backend |
|---|---|
| `TEST_BACKEND=sqlite` | SQLite (in-memory) |
| `TEST_BACKEND=postgres` | PostgreSQL |
| neither set, `DATABASE_URL` present | PostgreSQL |
| neither set, no `DATABASE_URL` | SQLite (in-memory) |

SQLite needs no server and no setup:

```bash
# Fast path - no PostgreSQL required. Unset DATABASE_URL so it cannot be picked up.
env -u DATABASE_URL TEST_BACKEND=sqlite cargo test
```

```bash
# PostgreSQL - required before changing any dialect-specific behaviour
export DATABASE_URL="postgres://postgres:postgres@localhost/djangors_test"
cargo test
```

Each SQLite test gets a fresh `sqlite::memory:` database, so isolation is automatic — there is no
shared state to clean up and no advisory lock to arbitrate. The pool is deliberately pinned to a
single connection: with a plain `sqlite::memory:` URL every pooled connection would otherwise be a
*separate* database, and setup DDL run on one connection would be invisible to the next query.

### Why both

SQLite is dramatically faster. Measured on a development machine, `djangors-admin`'s 32 tests take
**0.69s** on SQLite against **15.8s** on PostgreSQL.

It is not a full substitute. A handful of tests cover behaviour SQLite cannot express — transaction
isolation levels, `pg_advisory_lock`, `SKIP LOCKED` queue claiming, and PostgreSQL's width-strict
integer decoding. Those tests return early when `DATABASE_URL` is absent, each with a comment
naming the feature they need. **They report as passing on SQLite without executing**, so a green
SQLite run alone does not prove those paths still work.

Use SQLite for the fast inner development loop, and run against PostgreSQL before changing anything
dialect-specific and in CI.

---

## 3. Test Conventions

The suite is expected to stay green under **both** Postgres and SQLite. Conventions below keep it
that way.

### Where test code lives

Production `.rs` files contain no test code. Every `#[cfg(test)]` module lives in a **sibling
file**:

- For a crate's `lib.rs`, that is `src/tests.rs`, declared `#[cfg(test)] mod tests;` — see the
  bottom of `crates/djangors-admin/src/lib.rs`.
- For any other module, e.g. `src/queryset.rs`, it is `src/queryset_tests.rs`, declared with
  `#[cfg(test)] #[path = "queryset_tests.rs"] mod tests;` — see
  `crates/djangors-contrib-payments/src/bachs.rs`. The `#[path]` is required: without it Rust
  looks for `src/queryset/tests.rs`.

This keeps the production files readable, while the tests stay a **child module**, so they can
still reach private items. That is exactly why they are not in `crates/*/tests/` — an integration
test there only sees the crate's public API, and much of this suite exercises private functions.

### Writing tests that pass on both backends

Tests run as `cargo test --workspace` (Postgres, needs `DATABASE_URL`) and as
`env -u DATABASE_URL TEST_BACKEND=sqlite cargo test --workspace`. The rules below keep both green:

#### Build DDL from the dialect, never as a fixed string

`BIGSERIAL` and `TIMESTAMPTZ` are Postgres spellings; SQLite needs `INTEGER PRIMARY KEY
AUTOINCREMENT`. Use `db.dialect().auto_pk_type()` and `db.dialect().timestamp_type()`:

```rust,illustrative
let auto_pk = db.dialect().auto_pk_type();
let ts_type = db.dialect().timestamp_type();
db.conn().execute(
    &format!("CREATE TABLE widget (id {auto_pk}, created_at {ts_type} NOT NULL)"),
    &[],
)
.await
.unwrap();
```

SQLite is permissive enough to *accept* the Postgres text, so the `CREATE TABLE` succeeds and the
failure surfaces much later as an insert that returns no row — which is exactly how this bug hid
(see `create_table_sql` in `crates/djangors-contrib-payments/src/transaction_tests.rs`).

#### Quote identifiers through the dialect

`db.dialect().quote_ident("user")` — `user` is a reserved word in Postgres but not in SQLite, so
an unquoted column name passes on one backend and is a syntax error on the other:

```rust,illustrative
let user_col = db.dialect().quote_ident("user");
let create_profile = format!(
    "CREATE TABLE profile (id {auto_pk}, {user_col} BIGINT NOT NULL UNIQUE)"
);
```

#### Never use raw `sqlx` against `Database::pool()`

`pool()` panics outright on a SQLite handle (`"Attempted to access Postgres PgPool on a SQLite
Database handle"`), and `$1` placeholders are Postgres-only. Go through the ORM
(`Model::objects()`, `.save()`, `.count()`), which is already dialect-aware.

#### Never hardcode a connection string

Use `djangors_test::TestDatabase`, which honours `TEST_BACKEND` and `DATABASE_URL`. A hardcoded
`postgres://…@localhost/djangors_test` ignores both and can only pass against one specific machine.

#### Use `TestDatabase::isolated()` when a test creates its own tables

Several tests sharing one table name against a single Postgres database will race on
`DROP`/`CREATE` when run in parallel; `isolated()` gives each test its own database:

```rust,illustrative
let test_db = djangors_test::TestDatabase::isolated().await.unwrap();
let db = test_db.database();
```

`connect()` is fine for read-only work (see `TestDatabase::connect()` vs
`TestDatabase::isolated()` in `crates/djangors-test/src/lib.rs`).

### Checklist before adding a test

- Put the module in a sibling file (`src/tests.rs` or `src/{name}_tests.rs` with `#[path]`), never
  in the production `.rs`.
- Build every `CREATE TABLE` from `db.dialect().auto_pk_type()` / `timestamp_type()`, and quote
  identifiers with `quote_ident()`.
- Go through the ORM; never touch raw `pool()` or write `$1` placeholders by hand.
- Obtain the database via `djangors_test::TestDatabase`, never a hardcoded URL.
- Use `isolated()` when the test creates and drops tables; `connect()` only for read-only work.
- Run under **both** backends (`cargo test --workspace` and the SQLite env-invocation) before
  relying on it staying green.
