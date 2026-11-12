# Tutorial Part 5: Automated Testing

In Part 5, we write integration tests in Rust to test our endpoints, authentication, CSRF middleware, and database state transitions.

> [!NOTE]
> All test code in this part is adapted directly from [`examples/polls/tests/voting.rs`](file:///root/dev/Rango/examples/polls/tests/voting.rs).

---

## 1. Setting Up Integration Tests

Djangors applications leverage standard Rust integration test files in `tests/`. Create `tests/voting.rs`:

```rust,compile
use djangors_auth::{hash_password, User};
use djangors_core::middleware::{csrf_layer, security_headers_layer};
use djangors_core::router::RouterService;
use djangors_core::{Djangors, DjangorsSettings, Router};
use djangors_db::Database;
use djangors_orm::Model as _;
use djangors_sessions::{SessionLayer, SignedCookieStore};
use polls::models::{Choice, Question};
use polls::urls;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceBuilder;

static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn send_request(addr: SocketAddr, req: &str) -> String {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}
```

---

## 2. Launching Test Server & Executing Test Assertions

The test suite initializes test database tables, seeds model data, binds a Tokio TCP listener on an ephemeral port (`127.0.0.1:0`), and tests HTTP requests:

```rust,compile
# use djangors_auth::{hash_password, User};
# use djangors_core::middleware::{csrf_layer, security_headers_layer};
# use djangors_core::router::RouterService;
# use djangors_core::{Djangors, DjangorsSettings, Router};
# use djangors_db::Database;
# use djangors_sessions::{SessionLayer, SignedCookieStore};
# use polls::models::{Choice, Question};
# use polls::urls;
# use tokio::net::TcpListener;
# use tower::ServiceBuilder;
# static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
# async fn send_request(addr: std::net::SocketAddr, req: &str) -> String { String::new() }
# #[ignore]
#[tokio::test]
async fn test_polls_voting_integration() {
    let _guard = DB_MUTEX.lock().unwrap();

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = Database::connect(&config).await.unwrap();

    // Clean up and recreate schema for isolated test execution
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS polls_choice").execute(db.pool()).await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS polls_question").execute(db.pool()).await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user").execute(db.pool()).await;

    djangors_orm::sqlx::query(
        "CREATE TABLE auth_user (
            id BIGSERIAL PRIMARY KEY,
            username VARCHAR(150) NOT NULL,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined TIMESTAMPTZ NOT NULL,
            last_login TIMESTAMPTZ
        )",
    ).execute(db.pool()).await.unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE polls_question (
            id BIGSERIAL PRIMARY KEY,
            question_text VARCHAR(200) NOT NULL,
            pub_date TIMESTAMPTZ NOT NULL
        )",
    ).execute(db.pool()).await.unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE polls_choice (
            id BIGSERIAL PRIMARY KEY,
            question BIGINT NOT NULL REFERENCES polls_question(id) ON DELETE CASCADE,
            choice_text VARCHAR(200) NOT NULL,
            votes INTEGER NOT NULL DEFAULT 0
        )",
    ).execute(db.pool()).await.unwrap();

    // Seed test fixtures
    let now = chrono::Utc::now();
    let user = User {
        id: 0,
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: hash_password("correct_password").unwrap(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    user.save(&db).await.unwrap();

    let question = Question {
        id: 0,
        question_text: "What is your favorite color?".to_string(),
        pub_date: now,
    };
    let q_saved = question.save(&db).await.unwrap();

    let choice = Choice {
        id: 0,
        question: djangors_orm::ForeignKey::new(q_saved.id),
        choice_text: "Blue".to_string(),
        votes: 0,
    };
    let c_saved = choice.save(&db).await.unwrap();

    // Bind listener & spawn server task
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let settings = DjangorsSettings::new()
        .with_host("127.0.0.1")
        .with_port(addr.port())
        .with_debug(true);

    let router = urls::urls().with_state(db.clone());
    let router_service = RouterService::new(router, settings.debug);

    let service = ServiceBuilder::new()
        .layer(security_headers_layer())
        .layer(SessionLayer::new(SignedCookieStore::new(
            b"dev-only-secret-key-at-least-32-bytes-long-for-signing-cookies",
        )))
        .layer(csrf_layer())
        .service(router_service);

    let app = Djangors::new(settings, Router::new());
    tokio::spawn(async move {
        app.serve_service(listener, service).await.unwrap();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Test 1: GET / returns 200 OK
    let res = send_request(addr, "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await;
    assert!(res.contains("200 OK"));
    assert!(res.contains("What is your favorite color?"));
}
```

---

## What's Real vs. What Django Has That Djangors Doesn't Yet

> [!IMPORTANT]
> **Key Architecture Differences from Django:**
> - **Async Test Runner**: Tests use `
