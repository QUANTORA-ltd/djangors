#![deny(missing_docs)]
//! Small, in-process testing helpers for Djangors applications.

use bytes::Bytes;
use djangors_core::{AppState, Request, Router};
use djangors_db::{config::DatabaseConfig, Database, DbError};
use djangors_sessions::Session;
use hyper::http::{
    header::CONTENT_TYPE, Extensions, HeaderMap, HeaderValue, Method, StatusCode, Uri,
};
use std::sync::atomic::{AtomicU64, Ordering};
use tower::util::BoxCloneService;
use tower::ServiceExt;

type HyperRequest = hyper::Request<http_body_util::Full<Bytes>>;
type HyperResponse = hyper::Response<http_body_util::Full<Bytes>>;
type BoxService = BoxCloneService<HyperRequest, HyperResponse, tower::BoxError>;

#[derive(Clone)]
enum TestTarget {
    Router(Router),
    Service(BoxService),
}

/// An in-process client for a Djangors router.
#[derive(Clone)]
pub struct TestClient {
    target: TestTarget,
}

impl TestClient {
    /// Creates a new `TestClient` wrapping `router`.
    pub fn new(router: Router) -> Self {
        Self {
            target: TestTarget::Router(router),
        }
    }

    /// Creates a client around a cloneable Tower service, allowing requests to
    /// pass through the same middleware stack used by the application.
    pub fn from_service<S>(service: S) -> Self
    where
        S: tower::Service<HyperRequest, Response = HyperResponse> + Clone + Send + 'static,
        S::Error: Into<tower::BoxError>,
        S::Future: Send + 'static,
    {
        Self {
            target: TestTarget::Service(service.map_err(Into::into).boxed_clone()),
        }
    }

    /// Prepares a request with an arbitrary HTTP method.
    pub fn request(&self, method: Method, path: &str) -> RequestBuilder {
        RequestBuilder::new(self.target.clone(), method, path)
    }

    /// Prepares a GET request to `path`.
    pub fn get(&self, path: &str) -> RequestBuilder {
        self.request(Method::GET, path)
    }

    /// Prepares a POST request to `path` with urlencoded form pairs.
    pub fn post_form(&self, path: &str, pairs: &[(&str, &str)]) -> RequestBuilder {
        let body = serde_urlencoded::to_string(pairs).expect("form pairs should encode");
        let mut builder = self.request(Method::POST, path);
        builder.headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        builder.body = Bytes::from(body);
        builder
    }

    /// Prepares a JSON POST request.
    pub fn post_json<T: serde::Serialize + ?Sized>(&self, path: &str, body: &T) -> RequestBuilder {
        self.request(Method::POST, path).json(body)
    }

    /// Prepares a JSON PUT request.
    pub fn put_json<T: serde::Serialize + ?Sized>(&self, path: &str, body: &T) -> RequestBuilder {
        self.request(Method::PUT, path).json(body)
    }

    /// Prepares a JSON PATCH request.
    pub fn patch_json<T: serde::Serialize + ?Sized>(&self, path: &str, body: &T) -> RequestBuilder {
        self.request(Method::PATCH, path).json(body)
    }

    /// Prepares a DELETE request.
    pub fn delete(&self, path: &str) -> RequestBuilder {
        self.request(Method::DELETE, path)
    }
}

/// Builder for constructing in-process test HTTP requests.
pub struct RequestBuilder {
    target: TestTarget,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    extensions: Extensions,
    state: AppState,
}

impl RequestBuilder {
    fn new(target: TestTarget, method: Method, path: &str) -> Self {
        Self {
            target,
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

    /// Sets or replaces an HTTP request header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let name = hyper::http::header::HeaderName::from_bytes(name.as_bytes())
            .expect("header name should be valid");
        let value = HeaderValue::from_str(value).expect("header value should be valid");
        self.headers.insert(name, value);
        self
    }

    /// Sets the `Authorization: Bearer` header.
    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    /// Serializes a value as JSON and sets the JSON content type.
    pub fn json<T: serde::Serialize + ?Sized>(mut self, body: &T) -> Self {
        self.body = Bytes::from(serde_json::to_vec(body).expect("JSON body should serialize"));
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        self
    }

    /// Attaches state data to the test request.
    pub fn with_state<T: Send + Sync + 'static>(mut self, state: T) -> Self {
        self.state = self.state.insert(state);
        self
    }

    /// Sends the test request through the router and returns a [`TestResponse`].
    pub async fn send(self) -> TestResponse {
        let response = match self.target {
            TestTarget::Router(router) => {
                let request = Request::new(self.method, self.uri, self.headers, self.body)
                    .with_extensions(self.extensions)
                    .with_state(self.state);
                match router.handle(request).await {
                    Ok(response) => response.into_hyper(),
                    Err(error) => error.into_response().into_hyper(),
                }
            }
            TestTarget::Service(service) => {
                let mut request = hyper::Request::builder()
                    .method(self.method)
                    .uri(self.uri)
                    .body(http_body_util::Full::new(self.body))
                    .expect("test request should be valid");
                *request.headers_mut() = self.headers;
                *request.extensions_mut() = self.extensions;
                service
                    .oneshot(request)
                    .await
                    .expect("test service request should succeed")
            }
        };
        use http_body_util::BodyExt;
        let (parts, body) = response.into_parts();
        let body = body
            .collect()
            .await
            .expect("response body should be readable")
            .to_bytes();
        TestResponse {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }
}

/// Wrapper around an HTTP response for test assertions.
pub struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl TestResponse {
    /// Asserts that the response status matches `expected`.
    pub fn assert_status(self, expected: StatusCode) -> Self {
        assert_eq!(
            self.status,
            expected,
            "expected status {expected}, got {}; body: {}",
            self.status,
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
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Returns the HTTP status code of the response.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// Deserializes the response body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Deserializes the response body as a JSON value.
    pub fn json_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        self.json()
    }

    /// Returns the response headers.
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
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

#[cfg(test)]
mod tests;
