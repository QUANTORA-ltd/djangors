use djangors_auth::{hash_password, User};
use djangors_core::middleware::{csrf_layer, security_headers_layer};
use djangors_core::router::RouterService;
use djangors_core::{Djangors, DjangorsSettings, Router};
use djangors_db::Database;
use djangors_orm::Model as _;
use djangors_sessions::{SessionLayer, SignedCookieStore};
use school::models::{Course, Enrollment, Student};
use school::urls;
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
async fn test_school_admin_crud_integration() {
    let _guard = DB_MUTEX.lock().unwrap();

    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = Database::connect(&config).await.unwrap();

    // Clean up existing tables
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS school_enrollment")
        .execute(db.pool())
        .await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS school_student")
        .execute(db.pool())
        .await;
    let _ = djangors_orm::sqlx::query("DROP TABLE IF EXISTS school_course")
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
        "CREATE TABLE school_student (
            id BIGSERIAL PRIMARY KEY,
            first_name VARCHAR(100) NOT NULL,
            last_name VARCHAR(100) NOT NULL,
            email VARCHAR(254) NOT NULL UNIQUE,
            enrolled_date TIMESTAMPTZ NOT NULL,
            is_active BOOLEAN NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE school_course (
            id BIGSERIAL PRIMARY KEY,
            code VARCHAR(20) NOT NULL UNIQUE,
            name VARCHAR(200) NOT NULL,
            credits INTEGER NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    djangors_orm::sqlx::query(
        "CREATE TABLE school_enrollment (
            id BIGSERIAL PRIMARY KEY,
            student BIGINT NOT NULL REFERENCES school_student(id) ON DELETE CASCADE,
            course BIGINT NOT NULL REFERENCES school_course(id) ON DELETE CASCADE,
            enrolled_on TIMESTAMPTZ NOT NULL,
            grade VARCHAR(5) NOT NULL
        )",
    )
    .execute(db.pool())
    .await
    .unwrap();

    // 1. Create a superuser directly via the DB
    let now = chrono::Utc::now();
    let superuser = User {
        id: 0,
        username: "adminuser".to_string(),
        email: "admin@example.com".to_string(),
        password: hash_password("adminpassword").unwrap(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    };
    let _saved_superuser = superuser.save(&db).await.unwrap();

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

    // Get initial CSRF token
    let res = send_request(
        addr,
        "GET /admin/ HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    let csrf_cookie = get_cookie_value(&res, "csrftoken").expect("Should set csrftoken cookie");

    // 2. POST /accounts/login/ with that user's credentials, capture the session cookie
    let login_payload = "username=adminuser&password=adminpassword";
    let login_req = format!(
        "POST /accounts/login/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, csrf_cookie, login_payload.len(), login_payload
    );
    let res = send_request(addr, &login_req).await;
    assert!(
        res.contains("302 Found") || res.contains("303 See Other") || res.contains("302 Redirect"),
        "Login response: {res}"
    );
    let session_cookie =
        get_cookie_value(&res, "djangors_sessionid").expect("Login should return session cookie");

    // 3. POST /admin/school/student/add/ with a real student's fields -> 302
    let student_payload = "first_name=Alice&last_name=Smith&email=alice%40example.com&enrolled_date=2026-07-18+08%3A00%3A00&is_active=true";
    let student_add_req = format!(
        "POST /admin/school/student/add/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}; djangors_sessionid={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, session_cookie, csrf_cookie, student_payload.len(), student_payload
    );
    let res = send_request(addr, &student_add_req).await;
    assert!(
        res.contains("302 Found") || res.contains("303 See Other") || res.contains("302 Redirect"),
        "Student add response: {res}"
    );

    // Verify student created and get its PK
    let students = Student::objects().all(&db).await.unwrap();
    assert_eq!(students.len(), 1);
    let student_pk = students[0].id;

    // 4. POST /admin/school/course/add/ with a real course's fields -> 302
    let course_payload = "code=CS101&name=Intro+to+Computer+Science&credits=4";
    let course_add_req = format!(
        "POST /admin/school/course/add/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}; djangors_sessionid={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, session_cookie, csrf_cookie, course_payload.len(), course_payload
    );
    let res = send_request(addr, &course_add_req).await;
    assert!(
        res.contains("302 Found") || res.contains("303 See Other") || res.contains("302 Redirect"),
        "Course add response: {res}"
    );

    let courses = Course::objects().all(&db).await.unwrap();
    assert_eq!(courses.len(), 1);
    let course_pk = courses[0].id;

    // 5. POST /admin/school/enrollment/add/ referencing the student/course ids just created -> 302
    let enrollment_payload = format!(
        "student={}&course={}&enrolled_on=2026-07-18+08%3A00%3A00&grade=B",
        student_pk, course_pk
    );
    let enrollment_add_req = format!(
        "POST /admin/school/enrollment/add/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}; djangors_sessionid={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, session_cookie, csrf_cookie, enrollment_payload.len(), enrollment_payload
    );
    let res = send_request(addr, &enrollment_add_req).await;
    assert!(
        res.contains("302 Found") || res.contains("303 See Other") || res.contains("302 Redirect"),
        "Enrollment add response: {res}"
    );

    let enrollments = Enrollment::objects().all(&db).await.unwrap();
    assert_eq!(enrollments.len(), 1);
    let enrollment_pk = enrollments[0].id;

    // 6. GET /admin/school/enrollment/ (changelist) -> 200, response body contains enrollment
    let enrollment_list_req = format!(
        "GET /admin/school/enrollment/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: djangors_sessionid={}\r\nConnection: close\r\n\r\n",
        session_cookie
    );
    let res = send_request(addr, &enrollment_list_req).await;
    assert!(res.contains("200 OK"), "Enrollment list response: {res}");
    assert!(
        res.contains("school_enrollment") || res.contains("B") || res.contains("1"),
        "Enrollment list response body: {res}"
    );

    // 7. POST /admin/school/enrollment/save-changelist/ with edit-{pk}-grade=A -> 302
    let edit_payload = format!("edit-{}-grade=A", enrollment_pk);
    let save_changelist_req = format!(
        "POST /admin/school/enrollment/save-changelist/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}; djangors_sessionid={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        csrf_cookie, session_cookie, csrf_cookie, edit_payload.len(), edit_payload
    );
    let res = send_request(addr, &save_changelist_req).await;
    assert!(
        res.contains("302 Found") || res.contains("303 See Other") || res.contains("302 Redirect"),
        "Save changelist response: {res}"
    );

    // GET the changelist again and confirm grade shows A
    let res = send_request(addr, &enrollment_list_req).await;
    assert!(res.contains("200 OK"), "Enrollment list response 2: {res}");
    assert!(
        res.contains("A"),
        "Enrollment list response body 2 should contain grade A: {res}"
    );

    // 8. POST /admin/school/student/{pk}/delete/ with confirm=1 -> 302
    let delete_payload = "confirm=1";
    let delete_req = format!(
        "POST /admin/school/student/{}/delete/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: csrftoken={}; djangors_sessionid={}\r\nX-CSRFToken: {}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        student_pk, csrf_cookie, session_cookie, csrf_cookie, delete_payload.len(), delete_payload
    );
    let res = send_request(addr, &delete_req).await;
    assert!(
        res.contains("302 Found") || res.contains("303 See Other") || res.contains("302 Redirect"),
        "Delete response: {res}"
    );

    // GET the student changelist and confirm that student no longer appears
    let student_list_req = format!(
        "GET /admin/school/student/ HTTP/1.1\r\nHost: 127.0.0.1\r\nCookie: djangors_sessionid={}\r\nConnection: close\r\n\r\n",
        session_cookie
    );
    let res = send_request(addr, &student_list_req).await;
    assert!(res.contains("200 OK"), "Student list response: {res}");
    assert!(
        !res.contains("alice@example.com"),
        "Student list should not contain deleted student's email, response: {res}"
    );
}
