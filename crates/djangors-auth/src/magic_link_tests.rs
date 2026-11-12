use super::*;
use djangors_mail::InMemoryBackend;

async fn setup() -> Database {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();
    let create_user_sql = format!(
        "CREATE TABLE auth_user (
            id {auto_pk},
            username VARCHAR(150) NOT NULL UNIQUE,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined {ts_type} NOT NULL,
            last_login {ts_type}
        )"
    );
    db.conn().execute(&create_user_sql, &[]).await.unwrap();
    create_login_link_tables(&db).await.unwrap();
    db
}

async fn add_user(db: &Database, email: &str, active: bool) -> User {
    let user = User {
        id: 0,
        username: email.to_string(),
        email: email.to_string(),
        password: String::new(),
        is_active: active,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    };
    user.save(db).await.unwrap()
}

/// Pulls the raw token out of the sent email body.
fn token_from_mail(mail: &InMemoryBackend) -> String {
    let messages = mail.sent_messages();
    let body = &messages.last().expect("a mail was sent").body;
    let start = body.find("/login/magic/").unwrap() + "/login/magic/".len();
    let end = body[start..]
        .find(|c: char| c.is_whitespace())
        .map(|i| start + i)
        .unwrap_or(body.len());
    body[start..end].to_string()
}

#[tokio::test]
async fn login_link_works_once() {
    let db = setup().await;
    let user = add_user(&db, "magic@example.com", true).await;
    let mail = InMemoryBackend::default();

    send_login_link(
        &db,
        &mail,
        "  Magic@Example.com ",
        None,
        "https://app.test",
        LoginLinkPolicy::default(),
    )
    .await
    .unwrap();
    let token = token_from_mail(&mail);

    let consumed = consume_login_link(&db, &token).await.unwrap();
    assert_eq!(consumed.id, user.id);
    assert!(matches!(
        consume_login_link(&db, &token).await,
        Err(AuthError::InvalidToken)
    ));
}

#[tokio::test]
async fn unknown_email_gets_same_ok_and_no_mail() {
    let db = setup().await;
    let mail = InMemoryBackend::default();
    send_login_link(
        &db,
        &mail,
        "nobody@example.com",
        None,
        "https://app.test",
        LoginLinkPolicy::default(),
    )
    .await
    .unwrap();
    assert!(mail.sent_messages().is_empty());
}

#[tokio::test]
async fn inactive_user_gets_no_link() {
    let db = setup().await;
    add_user(&db, "off@example.com", false).await;
    let mail = InMemoryBackend::default();
    send_login_link(
        &db,
        &mail,
        "off@example.com",
        None,
        "https://app.test",
        LoginLinkPolicy::default(),
    )
    .await
    .unwrap();
    assert!(mail.sent_messages().is_empty());
}

#[tokio::test]
async fn expired_link_is_rejected() {
    let db = setup().await;
    add_user(&db, "late@example.com", true).await;
    let mail = InMemoryBackend::default();
    let policy = LoginLinkPolicy {
        ttl: Duration::ZERO,
        ..LoginLinkPolicy::default()
    };
    send_login_link(
        &db,
        &mail,
        "late@example.com",
        None,
        "https://app.test",
        policy,
    )
    .await
    .unwrap();
    let token = token_from_mail(&mail);
    assert!(matches!(
        consume_login_link(&db, &token).await,
        Err(AuthError::InvalidToken)
    ));
}

#[tokio::test]
async fn unknown_token_is_rejected() {
    let db = setup().await;
    assert!(matches!(
        consume_login_link(&db, "00ff").await,
        Err(AuthError::InvalidToken)
    ));
}

#[tokio::test]
async fn raw_token_is_not_stored() {
    let db = setup().await;
    add_user(&db, "store@example.com", true).await;
    let mail = InMemoryBackend::default();
    send_login_link(
        &db,
        &mail,
        "store@example.com",
        None,
        "https://app.test",
        LoginLinkPolicy::default(),
    )
    .await
    .unwrap();
    let token = token_from_mail(&mail);
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let row = conn
        .fetch_optional(
            &format!("SELECT token_hash FROM auth_login_link WHERE token_hash = {p1}"),
            &[BindValue::Text(token.clone())],
        )
        .await
        .unwrap();
    assert!(row.is_none(), "raw token must never be stored");
    let row = conn
        .fetch_optional(
            &format!("SELECT token_hash FROM auth_login_link WHERE token_hash = {p1}"),
            &[BindValue::Text(hash_login_token(&token))],
        )
        .await
        .unwrap();
    assert!(row.is_some());
}

#[tokio::test]
async fn sending_is_throttled_per_email() {
    let db = setup().await;
    add_user(&db, "busy@example.com", true).await;
    let mail = InMemoryBackend::default();
    let policy = LoginLinkPolicy {
        max_per_email: 2,
        ..LoginLinkPolicy::default()
    };
    for _ in 0..2 {
        send_login_link(
            &db,
            &mail,
            "busy@example.com",
            None,
            "https://app.test",
            policy,
        )
        .await
        .unwrap();
    }
    assert!(matches!(
        send_login_link(
            &db,
            &mail,
            "busy@example.com",
            None,
            "https://app.test",
            policy
        )
        .await,
        Err(AuthError::RateLimited)
    ));
}

#[tokio::test]
async fn sending_is_throttled_per_ip() {
    let db = setup().await;
    let mail = InMemoryBackend::default();
    let policy = LoginLinkPolicy {
        max_per_ip: 1,
        ..LoginLinkPolicy::default()
    };
    send_login_link(
        &db,
        &mail,
        "a@example.com",
        Some("203.0.113.9"),
        "https://app.test",
        policy,
    )
    .await
    .unwrap();
    assert!(matches!(
        send_login_link(
            &db,
            &mail,
            "b@example.com",
            Some("203.0.113.9"),
            "https://app.test",
            policy
        )
        .await,
        Err(AuthError::RateLimited)
    ));
}

#[test]
fn login_token_hash_is_stable_and_not_the_token() {
    let h = hash_login_token("abc");
    assert_eq!(h, hash_login_token("abc"));
    assert_ne!(h, "abc");
    assert_eq!(h.len(), 64);
}

#[tokio::test]
async fn get_confirm_does_not_consume_and_post_logs_user_in_once() {
    use bytes::Bytes;
    use djangors_core::state::AppState;
    use djangors_sessions::Session;
    use hyper::http::{Extensions, HeaderMap, Method, StatusCode, Uri};

    let db = setup().await;
    let user = add_user(&db, "view@example.com", true).await;
    let mail = InMemoryBackend::default();

    send_login_link(
        &db,
        &mail,
        "view@example.com",
        None,
        "https://app.test",
        LoginLinkPolicy::default(),
    )
    .await
    .unwrap();
    let token = token_from_mail(&mail);

    // GET shows the confirmation form without consuming the token.
    let mut get_params = PathParams::new();
    get_params.insert("token", &token);
    let get_req = Request::new(
        Method::GET,
        format!("/login/magic/{token}").parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    );
    let get_resp = magic_link_confirm_view(get_req, get_params).await.unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = std::str::from_utf8(get_resp.body()).unwrap();
    assert!(body.contains("method=\"post\""));

    // LoginLink row exists via ORM and is still unused.
    let links = LoginLink::objects().all(&db).await.unwrap();
    assert_eq!(links.len(), 1);
    assert!(links[0].used_at.is_none());
    assert_eq!(links[0].user.id, user.id);

    // POST consumes the token and logs the user into the session.
    let session = Session::new_empty();
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let post_req = Request::new(
        Method::POST,
        format!("/login/magic/{token}").parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(AppState::new().insert(db.clone()));
    let mut post_params = PathParams::new();
    post_params.insert("token", &token);
    let post_resp = magic_link_consume_view(post_req, post_params)
        .await
        .unwrap();
    assert_eq!(post_resp.status(), StatusCode::FOUND);
    assert_eq!(
        session.get::<i64>(crate::SESSION_USER_ID_KEY),
        Some(user.id)
    );

    // Replaying the POST fails with 400 Bad Request.
    let session2 = Session::new_empty();
    let mut extensions2 = Extensions::new();
    extensions2.insert(session2);
    let replay_req = Request::new(
        Method::POST,
        format!("/login/magic/{token}").parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions2)
    .with_state(AppState::new().insert(db));
    let mut replay_params = PathParams::new();
    replay_params.insert("token", &token);
    let err = magic_link_consume_view(replay_req, replay_params)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
}
