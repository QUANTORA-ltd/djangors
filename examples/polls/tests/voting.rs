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

fn get_cookie_value(response: &str, cookie_name: &str) -> Option<String> {
    for line in response.lines() {
        if line.to_lowercase().starts_with("set-cookie:") {
            let cookie_part = line.split_once(':')?.1.trim();
            if cookie_part.starts_with(cookie_name) {
                let parts: Vec<&str> = cookie_part.split(';').collect();
                if let Some(kv) = parts.first() {
                    if let Some((_, val)) = kv.split_once('=') {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_polls_voting_integration() {
    let _guard = DB_MUTEX.lock().unwrap();

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = Database::connect(&config).await.unwrap();

    // Clean up existing tables
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS polls_choice")
        .execute(db.pool())
        .await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS polls_question")
        .execute(db.pool())
        .await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS auth_user")
        .execute(db.pool())
        .await;

    // Create tables
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
    )
    .execute(db.pool())
    .await
    .unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE polls_question (
            id BIGSERIAL PRIMARY KEY,
            question_text VARCHAR(200) NOT NULL,
            pub_date TIMESTAMPTZ NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE polls_choice (
            id BIGSERIAL PRIMARY KEY,
            question BIGINT NOT NULL REFERENCES polls_question(id) ON DELETE CASCADE,
            choice_text VARCHAR(200) NOT NULL,
            votes INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // Seed test data
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
    let _saved_user = user.save(&db).await.unwrap();

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

    // Setup TCP server
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

    // Wait for server to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // 1. GET / remains accessible with no session/auth
    let res = send_request(
        addr,
        "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(res.contains("200 OK"), "GET / response: {res}");
    assert!(
        res.contains("What is your favorite color?"),
        "GET / response: {res}"
    );

    // 2. GET /1/ remains accessible with no session/auth
    let get_question_req = format!(
        "GET /{}/ HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        q_saved.id
    );
    let res = send_request(addr, &get_question_req).await;
    assert!(res.contains("200 OK"), "GET /1/ response: {res}");
    assert!(res.contains("Blue"), "GET /1/ response: {res}");

    // Extract CSRF token from the GET response
    let csrf_cookie = get_cookie_value(&res, "csrftoken").expect("Should set csrftoken cookie");

    // 3. Unauthenticated POST /1/vote/ returns 401, votes count unchanged
    let vote_payload = format!("choice={}", c_saved.id);
    let unauth_vote_req = format!(
        "POST /{}/vote/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        q_saved.id, csrf_cookie, csrf_cookie, vote_payload.len(), vote_payload
    );
    let res = send_request(addr, &unauth_vote_req).await;
    assert!(
        res.contains("401 Unauthorized"),
        "Expected 401, got response: {res}"
    );

    // Verify database votes count is still 0
    let db_choice = Choice::objects()
        .filter(djangors_orm::q!(id = c_saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(db_choice.votes, 0);

    // 4. POST /accounts/login/ with correct credentials returns redirect with Set-Cookie
    let login_payload = "username=testuser&password=correct_password";
    let login_req = format!(
        "POST /accounts/login/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, csrf_cookie, login_payload.len(), login_payload
    );
    let res = send_request(addr, &login_req).await;
    assert!(
        res.contains("303 See Other")
            || res.contains("302 Found")
            || res.contains("301 Moved Permanently")
            || res.contains("307 Temporary Redirect")
            || res.contains("308 Permanent Redirect")
            || res.contains("302 Redirect"),
        "Login response: {res}"
    );
    let session_cookie =
        get_cookie_value(&res, "djangors_sessionid").expect("Login should return session cookie");

    // 5. Authenticated POST /1/vote/ succeeds, votes count incremented
    let auth_vote_req = format!(
        "POST /{}/vote/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}; djangors_sessionid={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        q_saved.id, csrf_cookie, session_cookie, csrf_cookie, vote_payload.len(), vote_payload
    );
    let res = send_request(addr, &auth_vote_req).await;
    assert!(
        res.contains("303 See Other") || res.contains("302 Found") || res.contains("200 OK"),
        "Vote response: {res}"
    );

    // Verify database votes count is now 1
    let db_choice = Choice::objects()
        .filter(djangors_orm::q!(id = c_saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(db_choice.votes, 1);

    // 6. Unauthenticated GET /admin/ returns 401 Unauthorized
    let unauth_admin_req = "GET /admin/ HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    let res = send_request(addr, unauth_admin_req).await;
    assert!(
        res.contains("401 Unauthorized"),
        "GET /admin/ unauth response: {res}"
    );

    // 7. Create a staff user, log in to get session cookie, GET /admin/
    let staff_user = User {
        id: 0,
        username: "staffuser".to_string(),
        email: "staff@example.com".to_string(),
        password: hash_password("staff_password").unwrap(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    };
    let _saved_staff = staff_user.save(&db).await.unwrap();

    let staff_login_payload = "username=staffuser&password=staff_password";
    let staff_login_req = format!(
        "POST /accounts/login/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, csrf_cookie, staff_login_payload.len(), staff_login_payload
    );
    let res = send_request(addr, &staff_login_req).await;
    let staff_session_cookie = get_cookie_value(&res, "djangors_sessionid")
        .expect("Staff login should return session cookie");

    let auth_admin_req = format!(
        "GET /admin/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: djangors_sessionid={}\r\nConnection: close\r\n\r\n",
        staff_session_cookie
    );
    let res = send_request(addr, &auth_admin_req).await;
    assert!(res.contains("200 OK"), "GET /admin/ auth response: {res}");
    assert!(
        res.contains("polls.Question"),
        "GET /admin/ should list polls.Question, response: {res}"
    );
    assert!(
        res.contains("polls.Choice"),
        "GET /admin/ should list polls.Choice, response: {res}"
    );

    // 8. GET /admin/polls/question/ returns 200 and body contains seeded question's text
    let get_questions_list_req = format!(
        "GET /admin/polls/question/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: djangors_sessionid={}\r\nConnection: close\r\n\r\n",
        staff_session_cookie
    );
    let res = send_request(addr, &get_questions_list_req).await;
    assert!(
        res.contains("200 OK"),
        "GET /admin/polls/question/ response: {res}"
    );
    assert!(
        res.contains("What is your favorite color?"),
        "GET /admin/polls/question/ body should contain seeded question text, response: {res}"
    );
}
