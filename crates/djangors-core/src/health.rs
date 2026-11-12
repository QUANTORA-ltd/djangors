//! Liveness (`/healthz`) and readiness (`/readyz`) health-check endpoints.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hyper::StatusCode;

use crate::error::DjangorsError;
use crate::path_params::PathParams;
use crate::request::Request;
use crate::response::Response;
use crate::router::Router;

const DEFAULT_HEALTH_TIMEOUT: Duration = Duration::from_secs(3);

/// A dependency check executed by the `/readyz` readiness endpoint.
#[async_trait::async_trait]
pub trait HealthCheck: Send + Sync + 'static {
    /// Short identifier for this check in the `/readyz` JSON payload.
    fn name(&self) -> &str;

    /// Executes the check, returning `Ok(())` when healthy or a short,
    /// non-sensitive error description on failure.
    async fn check(&self) -> Result<(), String>;
}

/// Readiness check that executes `SELECT 1` against a [`djangors_db::Database`].
#[derive(Clone)]
pub struct DatabaseCheck(djangors_db::Database);

impl DatabaseCheck {
    /// Creates a [`DatabaseCheck`] for `db`.
    pub fn new(db: djangors_db::Database) -> Self {
        Self(db)
    }
}

#[async_trait::async_trait]
impl HealthCheck for DatabaseCheck {
    fn name(&self) -> &str {
        "database"
    }

    async fn check(&self) -> Result<(), String> {
        self.0
            .conn()
            .fetch_optional("SELECT 1", &[])
            .await
            .map(|_| ())
            .map_err(|err| {
                // The response stays generic so it can't leak connection details; the log
                // keeps the real cause for whoever is debugging the outage.
                tracing::warn!(error = %err, "readiness: database check failed");
                "database check failed".to_string()
            })
    }
}

/// Readiness check that performs a `set` / `get` / `delete` round-trip on a
/// [`djangors_cache::Cache`] backend using a 5-second TTL.
#[cfg(feature = "cache")]
#[derive(Clone)]
pub struct CacheCheck<C: djangors_cache::Cache + 'static>(Arc<C>);

#[cfg(feature = "cache")]
impl<C: djangors_cache::Cache + 'static> CacheCheck<C> {
    /// Creates a [`CacheCheck`] wrapping `cache`.
    pub fn new(cache: Arc<C>) -> Self {
        Self(cache)
    }
}

#[cfg(feature = "cache")]
#[async_trait::async_trait]
impl<C: djangors_cache::Cache + 'static> HealthCheck for CacheCheck<C> {
    fn name(&self) -> &str {
        "cache"
    }

    async fn check(&self) -> Result<(), String> {
        let key = format!("_healthz_{}", rand::random::<u64>());
        let ttl = Some(Duration::from_secs(5));
        self.0
            .set(&key, b"1".to_vec(), ttl)
            .await
            .map_err(|_| "cache check failed".to_string())?;
        let val = self
            .0
            .get(&key)
            .await
            .map_err(|_| "cache check failed".to_string())?;
        let _ = self.0.delete(&key).await;
        if val.as_deref() == Some(b"1".as_slice()) {
            Ok(())
        } else {
            Err("cache check failed".to_string())
        }
    }
}

/// Builder for configuring liveness and readiness checks on a [`Router`].
#[derive(Clone)]
pub struct HealthChecks {
    checks: Vec<Arc<dyn HealthCheck>>,
    timeout: Duration,
}

impl Default for HealthChecks {
    fn default() -> Self {
        Self {
            checks: Vec::new(),
            timeout: DEFAULT_HEALTH_TIMEOUT,
        }
    }
}

impl HealthChecks {
    /// Creates an empty [`HealthChecks`] builder with a 3-second per-check timeout.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a [`HealthCheck`] to run on `GET /readyz`.
    pub fn with<H: HealthCheck>(mut self, check: H) -> Self {
        self.checks.push(Arc::new(check));
        self
    }

    /// Sets the per-check timeout applied by `GET /readyz` (default 3 seconds).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

async fn healthz_handler(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let resp = Response::json(StatusCode::OK, &serde_json::json!({ "status": "ok" }))?;
    Ok(resp.header("cache-control", "no-store"))
}

impl Router {
    /// Registers `GET /healthz` (liveness) and `GET /readyz` (readiness) routes.
    ///
    /// - `GET /healthz` returns `200 {"status":"ok"}` without running any checks.
    /// - `GET /readyz` runs all registered checks concurrently with the configured
    ///   per-check timeout, returning `200` when all pass or `503` when any fail
    ///   or time out.
    ///
    /// Both responses include `Cache-Control: no-store`.
    pub fn health(self, checks: HealthChecks) -> Self {
        self.get("/healthz", healthz_handler).get(
            "/readyz",
            move |_req: Request, _params: PathParams| {
                let checks = checks.clone();
                async move {
                    let timeout = checks.timeout;
                    let futs = checks.checks.iter().map(|check| {
                        let check = Arc::clone(check);
                        async move {
                            let name = check.name().to_string();
                            let outcome = match tokio::time::timeout(timeout, check.check()).await {
                                Ok(Ok(())) => Ok(()),
                                Ok(Err(msg)) => Err(msg),
                                Err(_) => Err("timed out".to_string()),
                            };
                            (name, outcome)
                        }
                    });

                    let results = futures_util::future::join_all(futs).await;
                    let mut all_ok = true;
                    let mut map = BTreeMap::new();
                    for (name, outcome) in results {
                        match outcome {
                            Ok(()) => {
                                map.insert(name, "ok".to_string());
                            }
                            Err(msg) => {
                                all_ok = false;
                                map.insert(name, msg);
                            }
                        }
                    }

                    let (status, status_str) = if all_ok {
                        (StatusCode::OK, "ok")
                    } else {
                        (StatusCode::SERVICE_UNAVAILABLE, "error")
                    };

                    let resp = Response::json(
                        status,
                        &serde_json::json!({
                            "status": status_str,
                            "checks": map,
                        }),
                    )?;
                    Ok(resp.header("cache-control", "no-store"))
                }
            },
        )
    }
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
