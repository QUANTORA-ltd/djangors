use super::*;
use bytes::Bytes;
use hyper::StatusCode;
use std::{
    convert::Infallible,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tower::{service_fn, ServiceExt};

async fn exercise(cache: &dyn Cache) {
    assert_eq!(cache.get("missing").await.unwrap(), None);
    cache
        .set("key", b"value".to_vec(), Some(Duration::from_secs(5)))
        .await
        .unwrap();
    assert_eq!(cache.get("key").await.unwrap(), Some(b"value".to_vec()));
    assert_eq!(
        cache
            .get_or_set("key", None, || async { Ok(b"new".to_vec()) })
            .await
            .unwrap(),
        b"value"
    );
    cache.delete("key").await.unwrap();
    assert_eq!(
        cache
            .get_or_set("key", None, || async { Ok(b"new".to_vec()) })
            .await
            .unwrap(),
        b"new"
    );
    cache
        .set("ttl", b"gone".to_vec(), Some(Duration::from_millis(50)))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(cache.get("ttl").await.unwrap(), None);
}
#[tokio::test]
async fn in_memory_cache_operations_and_ttl() {
    exercise(&InMemoryCache::default()).await;
}

#[tokio::test]
async fn cache_add_is_insert_if_absent() {
    let cache = InMemoryCache::default();
    assert!(cache.add("once", b"one".to_vec(), None).await.unwrap());
    assert!(!cache.add("once", b"two".to_vec(), None).await.unwrap());
    assert_eq!(cache.get("once").await.unwrap(), Some(b"one".to_vec()));
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_database_cache_add_is_insert_if_absent() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = djangors_db::Database::connect(&config).await.unwrap();
    let cache = DatabaseCache::new(db);
    assert!(cache.add("once", b"one".to_vec(), None).await.unwrap());
    assert!(!cache.add("once", b"two".to_vec(), None).await.unwrap());
    assert_eq!(cache.get("once").await.unwrap(), Some(b"one".to_vec()));
}

#[tokio::test]
async fn idempotency_replays_and_rejects_reused_body() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let handler = service_fn(move |_req: Request<Full<Bytes>>| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(
                Response::builder()
                    .header("content-type", "text/plain")
                    .body(Full::new(Bytes::from_static(b"paid")))
                    .unwrap(),
            )
        }
    });
    let layer = IdempotencyLayer::new(Arc::new(InMemoryCache::default()), Duration::from_secs(60));
    let svc = layer.layer(handler);
    let request = |body: &'static [u8]| {
        Request::builder()
            .method("POST")
            .uri("/payments")
            .header("idempotency-key", "pay-123")
            .body(Full::new(Bytes::from_static(body)))
            .unwrap()
    };
    let first = svc.clone().oneshot(request(b"amount=10")).await.unwrap();
    let second = svc.clone().oneshot(request(b"amount=10")).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(second.headers().get("idempotent-replayed").unwrap(), "true");
    assert_eq!(
        second.body().clone().into_inner(),
        Some(Bytes::from_static(b"paid"))
    );
    let different = svc.clone().oneshot(request(b"amount=20")).await.unwrap();
    assert_eq!(different.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(first.status(), second.status());
}

#[tokio::test]
async fn idempotency_bypasses_unkeyed_requests_and_retries_server_errors() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let handler = service_fn(move |req: Request<Full<Bytes>>| {
        let c = c.clone();
        async move {
            let n = c.fetch_add(1, Ordering::SeqCst);
            let status = if req.method() == hyper::Method::GET || n < 2 {
                hyper::StatusCode::OK
            } else {
                hyper::StatusCode::INTERNAL_SERVER_ERROR
            };
            Ok::<_, Infallible>(
                Response::builder()
                    .status(status)
                    .body(Full::new(Bytes::from_static(b"handled")))
                    .unwrap(),
            )
        }
    });
    let svc = IdempotencyLayer::new(Arc::new(InMemoryCache::default()), Duration::from_secs(60))
        .layer(handler);
    let request = |method: &'static str, key: Option<&'static str>| {
        let mut builder = Request::builder().method(method).uri("/write");
        if let Some(key) = key {
            builder = builder.header("idempotency-key", key);
        }
        builder.body(Full::new(Bytes::new())).unwrap()
    };
    svc.clone()
        .oneshot(request("GET", Some("ignored")))
        .await
        .unwrap();
    svc.clone().oneshot(request("POST", None)).await.unwrap();
    svc.clone().oneshot(request("POST", None)).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    let first = svc
        .clone()
        .oneshot(request("POST", Some("retry")))
        .await
        .unwrap();
    assert_eq!(first.status(), hyper::StatusCode::INTERNAL_SERVER_ERROR);
    let second = svc
        .clone()
        .oneshot(request("POST", Some("retry")))
        .await
        .unwrap();
    assert_eq!(second.status(), hyper::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(count.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn concurrent_idempotency_requests_run_handler_once() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let handler = service_fn(move |_req: Request<Full<Bytes>>| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(30)).await;
            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"done"))))
        }
    });
    let svc = IdempotencyLayer::new(Arc::new(InMemoryCache::default()), Duration::from_secs(60))
        .layer(handler);
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/transfer")
            .header("idempotency-key", "transfer-1")
            .body(Full::new(Bytes::from_static(b"10")))
            .unwrap()
    };
    let (a, b) = tokio::join!(
        svc.clone().oneshot(request()),
        svc.clone().oneshot(request())
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!([hyper::StatusCode::OK, hyper::StatusCode::CONFLICT].contains(&a.status()));
    assert!([hyper::StatusCode::OK, hyper::StatusCode::CONFLICT].contains(&b.status()));
}

fn counting_handler(
    count: Arc<AtomicUsize>,
) -> tower::util::BoxCloneService<Request<Full<Bytes>>, Response<Full<Bytes>>, Infallible> {
    tower::util::BoxCloneService::new(service_fn(move |_req: Request<Full<Bytes>>| {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"done"))))
        }
    }))
}

#[tokio::test]
async fn idempotency_keys_are_scoped_per_caller() {
    let count = Arc::new(AtomicUsize::new(0));
    let svc = IdempotencyLayer::new(Arc::new(InMemoryCache::default()), Duration::from_secs(60))
        .layer(counting_handler(count.clone()));
    let request = |cookie: &'static str| {
        Request::builder()
            .method("POST")
            .uri("/orders")
            .header("idempotency-key", "same-key")
            .header("cookie", cookie)
            .body(Full::new(Bytes::from_static(b"{}")))
            .unwrap()
    };
    let alice = svc
        .clone()
        .oneshot(request("djangors_sessionid=alice"))
        .await
        .unwrap();
    let bob = svc
        .clone()
        .oneshot(request("djangors_sessionid=bob"))
        .await
        .unwrap();
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "each user's request must run"
    );
    assert!(bob.headers().get("idempotent-replayed").is_none());
    assert_eq!(alice.status(), StatusCode::OK);

    // A custom scope that ignores cookies makes the two requests the same operation.
    let shared = IdempotencyLayer::new(Arc::new(InMemoryCache::default()), Duration::from_secs(60))
        .with_scope(|_parts| b"tenant-1".to_vec())
        .layer(counting_handler(count.clone()));
    shared
        .clone()
        .oneshot(request("djangors_sessionid=alice"))
        .await
        .unwrap();
    let replayed = shared
        .clone()
        .oneshot(request("djangors_sessionid=bob"))
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert_eq!(
        replayed.headers().get("idempotent-replayed").unwrap(),
        "true"
    );
}

#[tokio::test]
async fn idempotency_in_progress_with_different_body_is_rejected() {
    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::default());
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (s, r) = (started.clone(), release.clone());
    let handler = service_fn(move |_req: Request<Full<Bytes>>| {
        let (s, r) = (s.clone(), r.clone());
        async move {
            s.notify_one();
            r.notified().await;
            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"done"))))
        }
    });
    let svc = IdempotencyLayer::new(cache, Duration::from_secs(60)).layer(handler);
    let request = |body: &'static [u8]| {
        Request::builder()
            .method("POST")
            .uri("/transfer")
            .header("idempotency-key", "k")
            .body(Full::new(Bytes::from_static(body)))
            .unwrap()
    };
    let first = tokio::spawn(svc.clone().oneshot(request(b"10")));
    started.notified().await;
    let other_body = svc.clone().oneshot(request(b"20")).await.unwrap();
    assert_eq!(other_body.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let same_body = svc.clone().oneshot(request(b"10")).await.unwrap();
    assert_eq!(same_body.status(), StatusCode::CONFLICT);
    release.notify_one();
    assert_eq!(first.await.unwrap().unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn dropped_request_releases_its_idempotency_key() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let handler = service_fn(move |_req: Request<Full<Bytes>>| {
        let c = c.clone();
        async move {
            // The first call never finishes, like a client that disconnects mid-request.
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                std::future::pending::<()>().await;
            }
            Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"done"))))
        }
    });
    let svc = IdempotencyLayer::new(
        Arc::new(InMemoryCache::default()),
        Duration::from_secs(3600),
    )
    .layer(handler);
    let request = || {
        Request::builder()
            .method("POST")
            .uri("/pay")
            .header("idempotency-key", "abandoned")
            .body(Full::new(Bytes::from_static(b"1")))
            .unwrap()
    };
    let abandoned = tokio::time::timeout(Duration::from_millis(50), svc.clone().oneshot(request()));
    assert!(
        abandoned.await.is_err(),
        "first request should still be running"
    );
    // The guard releases the key from a spawned task; give it a moment to run.
    let mut status = StatusCode::CONFLICT;
    for _ in 0..50 {
        status = svc.clone().oneshot(request()).await.unwrap().status();
        if status != StatusCode::CONFLICT {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn cache_layer_requires_response_opt_in() {
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let handler = service_fn(move |_req: Request<Full<Bytes>>| {
        let c = c.clone();
        async move {
            c.fetch_add(1, Ordering::SeqCst);
            let mut r = Response::new(Full::new(Bytes::from_static(b"cached")));
            r.extensions_mut().insert(CacheableResponse);
            Ok::<_, Infallible>(r)
        }
    });
    let mut svc = CacheLayer::new(Arc::new(InMemoryCache::default()), None).layer(handler);
    let request = || {
        Request::builder()
            .method("GET")
            .uri("/fragment?x=1")
            .body(Full::new(Bytes::new()))
            .unwrap()
    };
    assert_eq!(
        svc.ready()
            .await
            .unwrap()
            .call(request())
            .await
            .unwrap()
            .body()
            .clone()
            .into_inner(),
        Some(Bytes::from_static(b"cached"))
    );
    svc.ready().await.unwrap().call(request()).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
#[cfg(feature = "redis")]
#[tokio::test]
async fn redis_cache_operations_and_ttl() {
    exercise(&RedisCache::new("redis://127.0.0.1:6379").unwrap()).await;
}
#[tokio::test]
async fn database_cache_operations_and_ttl() {
    let sqlite = matches!(
        std::env::var("TEST_BACKEND").as_deref(),
        Ok(value) if value.eq_ignore_ascii_case("sqlite")
    );
    let config = if sqlite {
        // Each sqlite::memory: connection owns a separate database, so use a
        // single connection for the table setup and cache operations.
        djangors_db::DatabaseConfig::new("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
    } else {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors_test".into());
        djangors_db::DatabaseConfig::new(url)
    };
    let db = djangors_db::Database::connect(&config).await.unwrap();
    let cache = DatabaseCache::new(db);
    cache
        .db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_cache_entries", &[])
        .await
        .unwrap();
    exercise(&cache).await;

    // `add` is insert-if-absent on this backend (ON CONFLICT DO NOTHING), and an
    // expired entry does not block a new one.
    assert!(cache.add("add-key", b"one".to_vec(), None).await.unwrap());
    assert!(!cache.add("add-key", b"two".to_vec(), None).await.unwrap());
    assert_eq!(cache.get("add-key").await.unwrap(), Some(b"one".to_vec()));
    assert!(cache
        .add(
            "add-expiring",
            b"old".to_vec(),
            Some(Duration::from_millis(10))
        )
        .await
        .unwrap());
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(cache
        .add("add-expiring", b"new".to_vec(), None)
        .await
        .unwrap());
    assert_eq!(
        cache.get("add-expiring").await.unwrap(),
        Some(b"new".to_vec())
    );
    cache
        .db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_cache_entries", &[])
        .await
        .unwrap();
}
