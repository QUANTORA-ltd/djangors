use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use djangors::cache::{IdempotencyLayer, InMemoryCache};
use djangors::core::middleware::{cors_layer, security_headers_layer, CorsConfig};
use djangors::core::router::RouterService;
use djangors::core::versioning::DeprecationLayer;
use djangors::core::{Djangors, DjangorsSettings, Router};
use djangors::db::config::DatabaseConfig;
use djangors::db::Database;
use djangors::orm::Model as _;
use djangors::rest::{create_api_key, create_api_key_table};
use djangors::tasks::{create_task_table, QueuedTask};
use rest_api::models::{create_orders_table, Order};
use rest_api::urls::{api_versioning, urls};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceBuilder;

async fn send_request(addr: SocketAddr, req: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn test_rest_api_end_to_end() {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();

    create_orders_table(&db).await.unwrap();
    create_api_key_table(&db).await.unwrap();
    create_task_table(&db).await.unwrap();

    let (_ro_key, ro_raw) = create_api_key(&db, "reader", &["orders:read"], None)
        .await
        .unwrap();
    let (_rw_key, rw_raw) = create_api_key(&db, "writer", &["orders:read", "orders:write"], None)
        .await
        .unwrap();

    let cache = Arc::new(InMemoryCache::new(128));
    let router = urls(db.clone(), Arc::clone(&cache));
    let router_service = RouterService::new(router, true);

    let cors = cors_layer(&CorsConfig::new().allowed_origins(["https://app.example.com"])).unwrap();
    let service = ServiceBuilder::new()
        .layer(cors)
        .layer(security_headers_layer())
        .layer(DeprecationLayer::new(api_versioning()))
        .layer(IdempotencyLayer::new(cache, Duration::from_secs(60)))
        .service(router_service);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let settings = DjangorsSettings::new().with_debug(true);
    let app = Djangors::new(settings, Router::new());
    tokio::spawn(async move {
        let _ = app.serve_service(listener, service).await;
    });

    // 1. Health & readiness probes
    let res = send_request(
        addr,
        "GET /healthz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(res.contains("200 OK"), "healthz: {res}");
    assert!(res.contains(r#""status":"ok""#), "healthz body: {res}");

    let res = send_request(
        addr,
        "GET /readyz HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(res.contains("200 OK"), "readyz: {res}");
    assert!(res.contains(r#""database":"ok""#), "readyz db: {res}");
    assert!(res.contains(r#""cache":"ok""#), "readyz cache: {res}");

    // 2. OpenAPI spec includes version 2.0.0 and ApiKeyAuth security scheme
    let res = send_request(
        addr,
        "GET /openapi.json HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(res.contains("200 OK"), "openapi: {res}");
    assert!(res.contains("ApiKeyAuth"), "openapi securitySchemes: {res}");
    assert!(res.contains("2.0.0"), "openapi version: {res}");

    // 3. Deprecated /v1/orders/ emits Deprecation and Sunset headers, and requires API key
    let res = send_request(
        addr,
        &format!(
            "GET /v1/orders/ HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Api-Key: {ro_raw}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(res.contains("200 OK"), "v1 list: {res}");
    assert!(
        res.to_lowercase().contains("deprecation: true"),
        "v1 deprecation: {res}"
    );
    assert!(res.to_lowercase().contains("sunset:"), "v1 sunset: {res}");

    // 4. Read-only key gets 403 on POST /v2/orders/
    let body = r#"{"reference":"ORD-100","amount_cents":2500}"#;
    let res = send_request(
        addr,
        &format!(
            "POST /v2/orders/ HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Api-Key: {ro_raw}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    )
    .await;
    assert!(res.contains("403 Forbidden"), "ro create: {res}");

    // 5. Read-write key creates order + outbox task atomically, and idempotency key replays it
    let create_req = format!(
        "POST /v2/orders/ HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Api-Key: {rw_raw}\r\nIdempotency-Key: idem-100\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let res = send_request(addr, &create_req).await;
    assert!(res.contains("201 Created"), "rw create: {res}");

    let replay = send_request(addr, &create_req).await;
    assert!(replay.contains("201 Created"), "replay status: {replay}");
    assert!(
        replay.to_lowercase().contains("idempotent-replayed: true"),
        "replay header: {replay}"
    );

    let orders = Order::objects().all(&db).await.unwrap();
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].reference, "ORD-100");

    let tasks = QueuedTask::objects().all(&db).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task_name, "orders.send_receipt");

    // 6. Resilient upstream quote endpoint
    let res = send_request(
        addr,
        &format!(
            "GET /v2/quote/ HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Api-Key: {ro_raw}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(res.contains("200 OK"), "quote: {res}");
    assert!(res.contains(r#""shipping_cents":499"#), "quote body: {res}");
}
