#![deny(missing_docs)]
//! Small, in-process testing helpers for Djangors applications.

pub use djangors_macros::Factory;

use bytes::Bytes;
use djangors_core::{AppState, Request, Response, Router};
use djangors_db::{config::DatabaseConfig, Database, DbError};
use djangors_sessions::Session;
use hyper::http::{
    header::CONTENT_TYPE, Extensions, HeaderMap, HeaderValue, Method, StatusCode, Uri,
};
use std::sync::atomic::{AtomicU64, Ordering};

/// An in-process client for a Djangors router.
#[derive(Clone)]
pub struct TestClient {
    router: Router,
}

impl TestClient {
    /// Creates a new `TestClient` wrapping `router`.
    pub fn new(router: Router) -> Self {
        Self { router }
    }

    /// Prepares a GET request to `path`.
    pub fn get(&self, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.router.clone(), Method::GET, path)
    }

    /// Prepares a POST request to `path` with urlencoded form pairs.
    pub fn post_form(&self, path: &str, pairs: &[(&str, &str)]) -> RequestBuilder {
        let body = serde_urlencoded::to_string(pairs).expect("form pairs should encode");
        let mut builder = RequestBuilder::new(self.router.clone(), Method::POST, path);
        builder.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        builder.body = Bytes::from(body);
        builder
    }
}

/// Builder for constructing in-process test HTTP requests.
pub struct RequestBuilder {
    router: Router,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    extensions: Extensions,
    state: AppState,
}

impl RequestBuilder {
    fn new(router: Router, method: Method, path: &str) -> Self {
        Self {
            router,
            method,
            uri: path.parse().expect("test request path must be a valid URI"),
            headers: HeaderMap::new(),
            body: Bytes::new(),
            extensions: Extensions::new(),
            state: AppState::new(),
        }
    }

    /// Attaches a session extension to the test request.
    pub fn with_session(mut self, session: Session) -> Self {
        self.extensions.insert(session);
        self
    }

    /// Attaches state data to the test request.
    pub fn with_state<T: Send + Sync + 'static>(mut self, state: T) -> Self {
        self.state = self.state.insert(state);
        self
    }

    /// Sends the test request through the router and returns a [`TestResponse`].
    pub async fn send(self) -> TestResponse {
        let request = Request::new(self.method, self.uri, self.headers, self.body)
            .with_extensions(self.extensions)
            .with_state(self.state);
        let response = match self.router.handle(request).await {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        TestResponse { response }
    }
}

/// Wrapper around an HTTP response for test assertions.
pub struct TestResponse {
    response: Response,
}

impl TestResponse {
    /// Asserts that the response status matches `expected`.
    pub fn assert_status(self, expected: StatusCode) -> Self {
        assert_eq!(
            self.response.status(),
            expected,
            "expected status {expected}, got {}; body: {}",
            self.response.status(),
            self.body_str()
        );
        self
    }

    /// Asserts that the response body text contains `needle`.
    pub fn assert_contains(&self, needle: &str) -> &Self {
        let body = self.body_str();
        assert!(
            body.contains(needle),
            "expected response body to contain {needle:?}; body: {body}"
        );
        self
    }

    /// Returns the response body as a UTF-8 string.
    pub fn body_str(&self) -> String {
        String::from_utf8_lossy(self.response.body()).into_owned()
    }

    /// Returns the HTTP status code of the response.
    pub fn status(&self) -> StatusCode {
        self.response.status()
    }
}

/// Errors returned by test database operations.
#[derive(Debug, thiserror::Error)]
pub enum TestError {
    /// JSON deserialization failed.
    #[error("JSON deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    /// A database-level error occurred.
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    /// An ORM-level error occurred.
    #[error("ORM error: {0}")]
    Orm(#[from] djangors_orm::OrmError),
    /// A SQLx-level error occurred.
    #[error("SQLx error: {0}")]
    Sqlx(#[from] sqlx::Error),
    /// A generic error.
    #[error("{0}")]
    Other(String),
}

/// A unique database name counter, shared across the process.
static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Replaces the database name component in a `postgres://` URL string.
fn replace_db_name(url: &str, new_db: &str) -> String {
    let scheme_end = url.find("://").map(|i| i + 3).unwrap_or(0);
    let after_scheme = &url[scheme_end..];
    if let Some(relative_slash) = after_scheme.rfind('/') {
        let slash_pos = scheme_end + relative_slash;
        let after_slash = &url[slash_pos + 1..];
        match after_slash.find('?') {
            Some(q) => format!("{}/{}{}", &url[..slash_pos], new_db, &after_slash[q..]),
            None => format!("{}/{}", &url[..slash_pos], new_db),
        }
    } else {
        format!("{}/{}", url.trim_end_matches('/'), new_db)
    }
}

fn generate_db_name() -> String {
    let pid = std::process::id();
    let count = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("djangors_test_{pid}_{count}_{nanos}")
}

/// Which backend a test database is provisioned against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestBackend {
    /// PostgreSQL database backend.
    Postgres,
    /// SQLite database backend (in-memory).
    Sqlite,
}

impl TestBackend {
    /// Detects the active test backend based on environment variables:
    /// 1. `TEST_BACKEND=sqlite` -> `TestBackend::Sqlite`
    /// 2. `TEST_BACKEND=postgres` -> `TestBackend::Postgres`
    /// 3. If `TEST_BACKEND` is unset: `DATABASE_URL` set -> `Postgres`, unset -> `Sqlite`.
    pub fn current() -> Self {
        if let Ok(var) = std::env::var("TEST_BACKEND") {
            match var.to_lowercase().as_str() {
                "sqlite" => return TestBackend::Sqlite,
                "postgres" => return TestBackend::Postgres,
                _ => {}
            }
        }
        if std::env::var("DATABASE_URL").is_ok() {
            TestBackend::Postgres
        } else {
            TestBackend::Sqlite
        }
    }
}

/// A thin database fixture. It intentionally does not provide transactional rollback:
/// ORM querysets currently require `&Database` and execute directly through its pool.
///
/// The primary cleanup mechanism for isolated databases is the explicit
/// [`TestDatabase::cleanup`] method. As a defensive fallback, the `Drop` impl will attempt
/// a best-effort `DROP DATABASE` via an ambient tokio runtime handle if one is available.
/// Always prefer calling `cleanup().await` explicitly in test code.
pub struct TestDatabase {
    database: Database,
    db_name: Option<String>,
    admin_base_url: String,
    is_isolated: bool,
}

impl TestDatabase {
    /// Provisions a database for one test, honouring `backend`.
    pub async fn new_for_backend(backend: TestBackend) -> Result<Self, DbError> {
        match backend {
            TestBackend::Sqlite => {
                // SQLite in-memory database: every pooled connection is a separate database when using sqlite::memory:.
                // Therefore max_connections MUST be set to 1 so that setup DDL executed on one connection
                // is visible to subsequent queries on the same pooled connection handle.
                let config = DatabaseConfig::new("sqlite::memory:".to_string())
                    .max_connections(1)
                    .min_connections(1);
                let database = Database::connect(&config).await?;
                Ok(Self {
                    database,
                    db_name: None,
                    admin_base_url: String::new(),
                    is_isolated: true,
                })
            }
            TestBackend::Postgres => {
                let url = std::env::var("DATABASE_URL").map_err(|_| {
                    DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
                })?;
                Self::isolated_url(&url).await
            }
        }
    }

    /// Connects to the database specified by the environment configuration (`TEST_BACKEND` or `DATABASE_URL`).
    pub async fn connect() -> Result<Self, DbError> {
        match TestBackend::current() {
            TestBackend::Sqlite => Self::new_for_backend(TestBackend::Sqlite).await,
            TestBackend::Postgres => {
                let url = std::env::var("DATABASE_URL").map_err(|_| {
                    DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
                })?;
                Self::connect_url(&url).await
            }
        }
    }

    /// Connects to a database at the specified URL string.
    pub async fn connect_url(url: &str) -> Result<Self, DbError> {
        Ok(Self {
            database: Database::connect(&DatabaseConfig::new(url.to_string())).await?,
            db_name: None,
            admin_base_url: String::new(),
            is_isolated: false,
        })
    }

    /// Returns a reference to the underlying [`Database`].
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Executes raw DDL SQL to create a table.
    pub async fn create_table(&self, sql: &str) -> Result<(), sqlx::Error> {
        self.database.conn().execute(sql, &[]).await.map(|_| ())
    }

    /// Drops a table by name if it exists.
    pub async fn drop_table(&self, name: &str) -> Result<(), sqlx::Error> {
        let sql = format!("DROP TABLE IF EXISTS \"{name}\"");
        self.database.conn().execute(&sql, &[]).await.map(|_| ())
    }

    /// Drops each table in `tables` sequentially.
    pub async fn reset(&self, tables: &[&str]) -> Result<(), sqlx::Error> {
        for table in tables {
            self.drop_table(table).await?;
        }
        Ok(())
    }

    /// Creates a uniquely-named throwaway database, connects to it, and returns a
    /// [`TestDatabase`] backed by that isolated database.
    ///
    /// The base connection is determined by `TEST_BACKEND` or `DATABASE_URL`.
    pub async fn isolated() -> Result<Self, DbError> {
        match TestBackend::current() {
            TestBackend::Sqlite => Self::new_for_backend(TestBackend::Sqlite).await,
            TestBackend::Postgres => {
                let url = std::env::var("DATABASE_URL").map_err(|_| {
                    DbError::ConnectionFailed("DATABASE_URL environment variable is not set".into())
                })?;
                Self::isolated_url(&url).await
            }
        }
    }

    /// Like [`TestDatabase::isolated`] but connects to the given URL string for the
    /// base connection instead of reading `DATABASE_URL`.
    pub async fn isolated_url(url: &str) -> Result<Self, DbError> {
        if matches!(
            djangors_db::Dialect::from_url(url),
            djangors_db::Dialect::Sqlite
        ) {
            let config = DatabaseConfig::new("sqlite::memory:".to_string())
                .max_connections(1)
                .min_connections(1);
            let database = Database::connect(&config).await?;
            return Ok(Self {
                database,
                db_name: None,
                admin_base_url: String::new(),
                is_isolated: true,
            });
        }

        let db_name = generate_db_name();

        let admin_url = replace_db_name(url, "postgres");
        let admin_db = Database::connect(&DatabaseConfig::new(admin_url)).await?;

        let create_sql = format!("CREATE DATABASE \"{db_name}\"");
        sqlx::query(sqlx::AssertSqlSafe(create_sql))
            .execute(admin_db.pool())
            .await
            .map_err(|e| DbError::ConnectionFailed(e.to_string()))?;

        drop(admin_db);

        let isolated_url = replace_db_name(url, &db_name);
        let database = Database::connect(&DatabaseConfig::new(isolated_url)).await?;

        Ok(Self {
            database,
            db_name: Some(db_name),
            admin_base_url: url.to_string(),
            is_isolated: true,
        })
    }

    /// Explicitly drops the throwaway database backing this [`TestDatabase`].
    ///
    /// This is the **primary cleanup mechanism** for isolated databases.
    pub async fn cleanup(self) -> Result<(), TestError> {
        if self.database.dialect() == djangors_db::Dialect::Sqlite {
            // SQLite in-memory database dies automatically with the connection handle.
            return Ok(());
        }

        let db_name = match self.db_name.as_ref() {
            Some(name) => name.clone(),
            None => return Ok(()),
        };
        let admin_base_url = self.admin_base_url.clone();

        drop(self);

        let admin_url = replace_db_name(&admin_base_url, "postgres");
        let admin_db = Database::connect(&DatabaseConfig::new(admin_url))
            .await
            .map_err(TestError::Db)?;

        sqlx::query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
        )
        .bind(&db_name)
        .execute(admin_db.pool())
        .await?;

        let drop_sql = format!("DROP DATABASE IF EXISTS \"{db_name}\"");
        sqlx::query(sqlx::AssertSqlSafe(drop_sql))
            .execute(admin_db.pool())
            .await?;

        Ok(())
    }

    /// Loads rows into a freshly-created table from a JSON string.
    ///
    /// Deserializes `json` as a `Vec<T>` and calls [`QuerySet::bulk_create`] to insert
    /// every element. `T` must implement both [`djangors_orm::Model`] (to satisfy the ORM
    /// layer) and [`serde::de::DeserializeOwned`] (to parse the JSON).
    ///
    /// Returns the deserialized items. Primary-key fields that are auto-generated will
    /// not be populated in the returned values; use the normal `QuerySet` API to query
    /// the rows back from the database if PKs are needed.
    pub async fn load_fixtures<
        T: djangors_orm::Model + djangors_orm::FromRow + serde::de::DeserializeOwned,
    >(
        &self,
        json: &str,
    ) -> Result<Vec<T>, TestError> {
        let items: Vec<T> = serde_json::from_str(json)?;
        djangors_orm::QuerySet::<T>::bulk_create(self.database(), &items).await?;
        Ok(items)
    }
}

impl Drop for TestDatabase {
    /// Defensive fallback: if this is an isolated database and a tokio runtime handle
    /// is available, spawn a best-effort `DROP DATABASE`. The explicit
    /// [`TestDatabase::cleanup`] method is the primary mechanism; this is only a
    /// safety net for tests that forget to call it.
    fn drop(&mut self) {
        if !self.is_isolated || self.database.dialect() == djangors_db::Dialect::Sqlite {
            return;
        }
        if let Some(ref db_name) = self.db_name {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                let db_name = db_name.clone();
                let admin_base_url = self.admin_base_url.clone();
                handle.spawn(async move {
                    let admin_url = replace_db_name(&admin_base_url, "postgres");
                    if let Ok(admin_db) =
                        Database::connect(&DatabaseConfig::new(admin_url)).await
                    {
                        let _ = sqlx::query(
                            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid <> pg_backend_pid()",
                        )
                        .bind(&db_name)
                        .execute(admin_db.pool())
                        .await;
                        let drop_sql = format!("DROP DATABASE IF EXISTS \"{db_name}\"");
                        let _ = sqlx::query(sqlx::AssertSqlSafe(drop_sql)).execute(admin_db.pool()).await;
                    }
                });
            }
        }
    }
}

/// A held Postgres session-level advisory lock, acquired via
/// [`acquire_cross_process_lock`]. Released by calling [`CrossProcessLock::release`]
/// explicitly (Rust can't run async code in `Drop`, so this can't release itself
/// automatically the way [`TestDatabase`]'s in-process guards can) - an
/// un-released lock is held until its underlying connection closes, which
/// happens naturally when the test process exits, but explicit release lets a
/// long-running process (a real `dj test` run covering many tests) free the
/// lock promptly instead of holding it for the rest of the run.
pub struct CrossProcessLock {
    conn: sqlx::pool::PoolConnection<sqlx::Postgres>,
    key_name: String,
}

impl CrossProcessLock {
    /// Releases the lock. The connection this lock was acquired on must be the
    /// one that releases it - `pg_advisory_unlock` is a per-session operation,
    /// not per-pool - which is exactly why this type holds a single dedicated
    /// [`sqlx::pool::PoolConnection`] instead of borrowing from the pool anew for
    /// each query the way most of this codebase's other queries do.
    pub async fn release(mut self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
            .bind(&self.key_name)
            .execute(&mut *self.conn)
            .await?;
        Ok(())
    }
}

/// Acquires a Postgres session-level advisory lock keyed by `name`, blocking
/// until any other session holding the same-named lock releases it.
///
/// Unlike an in-process `std::sync::Mutex`/`tokio::sync::Mutex` (which only
/// coordinates tasks within one compiled test *binary*), this coordinates
/// across entirely separate OS processes connected to the same
/// Postgres server - the actual mechanism needed to prevent two different
/// crates' test binaries (e.g. `djangors-admin` and `djangors-auth`, both racing
/// to `DROP`/`CREATE` the same `auth_user` table) from colliding when `cargo
/// test --workspace` runs them as concurrent processes. Confirmed reproducible
/// without this: running `djangors-admin` and `djangors-auth`'s suites
/// concurrently produced a real `relation "auth_user" already exists` (42P07)
/// in one and `relation "auth_user" does not exist` (42P01) in the other.
///
/// Deliberately holds a single dedicated connection (via
/// [`sqlx::Pool::acquire`]) for the lock's entire lifetime rather than using
/// `db.pool()` directly - a session-level advisory lock is tied to the specific
/// connection that acquired it, and a connection pool hands out whichever
/// connection happens to be free for each query, so acquiring/releasing through
/// the pool directly would not reliably lock/unlock the same session.
pub async fn acquire_cross_process_lock(
    db: &Database,
    name: &str,
) -> Result<CrossProcessLock, sqlx::Error> {
    let mut conn = db.pool().acquire().await?;
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(name)
        .execute(&mut *conn)
        .await?;
    Ok(CrossProcessLock {
        conn,
        key_name: name.to_string(),
    })
}

/// Asserts that `fut` executes exactly `expected` SQL queries against the database.
///
/// Wraps [`djangors_db::with_query_log`] and panics with a detailed dump of every
/// executed SQL statement if the actual query count differs from `expected`.
pub async fn assert_num_queries<F, T>(expected: usize, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let (result, stats) = djangors_db::with_query_log(fut).await;
    let actual = stats.total_queries();
    if actual != expected {
        let mut dump = String::new();
        for (i, q) in stats.queries.iter().enumerate() {
            dump.push_str(&format!("  {}. {}\n", i + 1, q.sql));
        }
        panic!("assert_num_queries failed: expected {expected} queries, got {actual}:\n{dump}");
    }
    result
}

/// A live HTTP server bound to `127.0.0.1:0` (ephemeral port) for end-to-end tests.
///
/// Automatically shuts down its background server task when dropped.
pub struct LiveServer {
    addr: std::net::SocketAddr,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<()>,
}

impl LiveServer {
    /// Binds an ephemeral local port on `127.0.0.1:0` and starts serving `router` in a
    /// background Tokio task.
    pub async fn start(router: Router) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("LiveServer failed to bind 127.0.0.1:0");
        let addr = listener
            .local_addr()
            .expect("LiveServer failed to read local_addr");
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let app = djangors_core::Djangors::new(djangors_core::DjangorsSettings::default(), router);
        let handle = tokio::spawn(async move {
            let _ = app
                .serve_with_shutdown(listener, async move {
                    let _ = rx.await;
                })
                .await;
        });
        Self {
            addr,
            shutdown_tx: Some(tx),
            handle,
        }
    }

    /// Returns the bound local socket address.
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// Returns the base URL of the running server (e.g. `"http://127.0.0.1:54321"`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Builds a full URL for `path` against this live server.
    pub fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("http://{}{}", self.addr, path)
        } else {
            format!("http://{}/{}", self.addr, path)
        }
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.handle.abort();
    }
}

/// Factory for constructing [`djangors_core::Request`] objects directly in unit tests
/// without instantiating a [`Router`].
///
/// Mirrors Django's `django.test.RequestFactory`.
#[derive(Debug, Default, Clone)]
pub struct RequestFactory {
    default_headers: HeaderMap,
}

impl RequestFactory {
    /// Creates a new [`RequestFactory`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a default header applied to every request built by this factory.
    pub fn with_default_header(mut self, name: &str, value: &str) -> Self {
        let header_name = hyper::http::HeaderName::from_bytes(name.as_bytes())
            .expect("header name must be valid HTTP token");
        let header_value = HeaderValue::from_str(value).expect("header value must be valid ASCII");
        self.default_headers.insert(header_name, header_value);
        self
    }

    /// Starts building a request with an arbitrary HTTP `method` and `path`.
    pub fn request(&self, method: Method, path: &str) -> FactoryRequestBuilder {
        FactoryRequestBuilder {
            method,
            uri: path.parse().expect("request path must be a valid URI"),
            headers: self.default_headers.clone(),
            body: Bytes::new(),
            extensions: Extensions::new(),
            state: AppState::new(),
        }
    }

    /// Starts building a `GET` request.
    pub fn get(&self, path: &str) -> FactoryRequestBuilder {
        self.request(Method::GET, path)
    }

    /// Starts building a `POST` request.
    pub fn post(&self, path: &str) -> FactoryRequestBuilder {
        self.request(Method::POST, path)
    }

    /// Starts building a `PUT` request.
    pub fn put(&self, path: &str) -> FactoryRequestBuilder {
        self.request(Method::PUT, path)
    }

    /// Starts building a `PATCH` request.
    pub fn patch(&self, path: &str) -> FactoryRequestBuilder {
        self.request(Method::PATCH, path)
    }

    /// Starts building a `DELETE` request.
    pub fn delete(&self, path: &str) -> FactoryRequestBuilder {
        self.request(Method::DELETE, path)
    }

    /// Starts building a `HEAD` request.
    pub fn head(&self, path: &str) -> FactoryRequestBuilder {
        self.request(Method::HEAD, path)
    }
}

/// Builder produced by [`RequestFactory`] for constructing a standalone [`Request`].
pub struct FactoryRequestBuilder {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    extensions: Extensions,
    state: AppState,
}

impl FactoryRequestBuilder {
    /// Adds a header to the request.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let header_name = hyper::http::HeaderName::from_bytes(name.as_bytes())
            .expect("header name must be valid HTTP token");
        let header_value = HeaderValue::from_str(value).expect("header value must be valid ASCII");
        self.headers.insert(header_name, header_value);
        self
    }

    /// Sets the raw request body bytes.
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Serializes `value` as JSON and sets `Content-Type: application/json`.
    pub fn json<T: serde::Serialize>(mut self, value: &T) -> Self {
        let bytes = serde_json::to_vec(value).expect("request JSON body should serialize");
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        self.body = Bytes::from(bytes);
        self
    }

    /// Encodes `pairs` as `application/x-www-form-urlencoded` form data.
    pub fn form(mut self, pairs: &[(&str, &str)]) -> Self {
        let encoded = serde_urlencoded::to_string(pairs).expect("form pairs should encode");
        self.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        self.body = Bytes::from(encoded);
        self
    }

    /// Attaches a [`Session`] extension to the request.
    pub fn with_session(mut self, session: Session) -> Self {
        self.extensions.insert(session);
        self
    }

    /// Attaches an arbitrary extension to the request.
    pub fn with_extension<T: Clone + Send + Sync + 'static>(mut self, ext: T) -> Self {
        self.extensions.insert(ext);
        self
    }

    /// Attaches application state to the request.
    pub fn with_state<T: Send + Sync + 'static>(mut self, state: T) -> Self {
        self.state = self.state.insert(state);
        self
    }

    /// Finalizes and returns the [`Request`].
    pub fn build(self) -> Request {
        Request::new(self.method, self.uri, self.headers, self.body)
            .with_extensions(self.extensions)
            .with_state(self.state)
    }
}

static ENV_OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that overrides environment variables (such as `DJANGORS_*` settings)
/// for the duration of a test and restores their previous values on [`Drop`].
///
/// Serializes concurrent tests in the same process via a global mutex so
/// environment mutations do not race.
pub struct SettingsOverride {
    previous: Vec<(String, Option<String>)>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl SettingsOverride {
    /// Applies a list of `(key, value)` environment overrides (`Some(v)` sets,
    /// `None` removes) and returns an RAII guard that restores them on drop.
    pub fn new(vars: &[(&str, Option<&str>)]) -> Self {
        let lock = ENV_OVERRIDE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut previous = Vec::with_capacity(vars.len());
        for &(key, val) in vars {
            previous.push((key.to_string(), std::env::var(key).ok()));
            match val {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        Self {
            previous,
            _lock: lock,
        }
    }

    /// Convenience constructor overriding a single environment variable.
    pub fn set(key: &str, value: &str) -> Self {
        Self::new(&[(key, Some(value))])
    }

    /// Convenience constructor temporarily removing a single environment variable.
    pub fn remove(key: &str) -> Self {
        Self::new(&[(key, None)])
    }
}

impl Drop for SettingsOverride {
    fn drop(&mut self) {
        // Restore in reverse order in case the same key appeared more than once.
        for (key, prev) in self.previous.iter().rev() {
            match prev {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Overrides environment variables for the lifetime of the returned [`SettingsOverride`] guard.
///
/// Equivalent to [`SettingsOverride::new`].
pub fn override_env(vars: &[(&str, Option<&str>)]) -> SettingsOverride {
    SettingsOverride::new(vars)
}

#[cfg(test)]
mod tests;
