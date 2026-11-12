use super::*;
use bytes::Bytes;
use djangors_db::{Database, DatabaseConfig};
use hyper::http::{HeaderMap, Method, Uri};
use std::sync::atomic::{AtomicUsize, Ordering};

struct MockCheck {
    name: &'static str,
    result: Result<(), &'static str>,
    calls: Arc<AtomicUsize>,
    delay: Option<Duration>,
}

#[async_trait::async_trait]
impl HealthCheck for MockCheck {
    fn name(&self) -> &str {
        self.name
    }

    async fn check(&self) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.delay {
            tokio::time::sleep(delay).await;
        }
        self.result.map_err(ToString::to_string)
    }
}

fn get_req(path: &str) -> Request {
    Request::new(
        Method::GET,
        path.parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
}

#[tokio::test]
async fn healthz_never_runs_checks_and_readyz_passes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let router = Router::new().health(HealthChecks::new().with(MockCheck {
        name: "redis",
        result: Ok(()),
        calls: Arc::clone(&calls),
        delay: None,
    }));

    let health_resp = router.handle(get_req("/healthz")).await.unwrap();
    assert_eq!(health_resp.status(), StatusCode::OK);
    assert_eq!(
        health_resp.headers().get("cache-control").unwrap(),
        "no-store"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let health_json: serde_json::Value = serde_json::from_slice(health_resp.body()).unwrap();
    assert_eq!(health_json["status"], "ok");

    let ready_resp = router.handle(get_req("/readyz")).await.unwrap();
    assert_eq!(ready_resp.status(), StatusCode::OK);
    assert_eq!(
        ready_resp.headers().get("cache-control").unwrap(),
        "no-store"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let ready_json: serde_json::Value = serde_json::from_slice(ready_resp.body()).unwrap();
    assert_eq!(ready_json["status"], "ok");
    assert_eq!(ready_json["checks"]["redis"], "ok");
}

#[tokio::test]
async fn readyz_returns_503_when_check_fails() {
    let router = Router::new().health(
        HealthChecks::new()
            .with(MockCheck {
                name: "ok_svc",
                result: Ok(()),
                calls: Arc::new(AtomicUsize::new(0)),
                delay: None,
            })
            .with(MockCheck {
                name: "broken_svc",
                result: Err("connection refused"),
                calls: Arc::new(AtomicUsize::new(0)),
                delay: None,
            }),
    );

    let resp = router.handle(get_req("/readyz")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert_eq!(json["status"], "error");
    assert_eq!(json["checks"]["ok_svc"], "ok");
    assert_eq!(json["checks"]["broken_svc"], "connection refused");
}

#[tokio::test]
async fn readyz_times_out_slow_check() {
    let router = Router::new().health(HealthChecks::new().timeout(Duration::from_millis(20)).with(
        MockCheck {
            name: "slow_svc",
            result: Ok(()),
            calls: Arc::new(AtomicUsize::new(0)),
            delay: Some(Duration::from_secs(2)),
        },
    ));

    let start = tokio::time::Instant::now();
    let resp = router.handle(get_req("/readyz")).await.unwrap();
    let elapsed = start.elapsed();

    assert!(elapsed < Duration::from_millis(500));
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let json: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert_eq!(json["status"], "error");
    assert_eq!(json["checks"]["slow_svc"], "timed out");
}

#[tokio::test]
async fn database_check_against_sqlite_in_memory() {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    let router = Router::new().health(HealthChecks::new().with(DatabaseCheck::new(db)));

    let resp = router.handle(get_req("/readyz")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["checks"]["database"], "ok");
}
