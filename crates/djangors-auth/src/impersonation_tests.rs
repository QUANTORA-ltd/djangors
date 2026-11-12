use super::*;
use bytes::Bytes;
use djangors_core::state::AppState;
use djangors_core::PathParams;
use hyper::http::{Extensions, HeaderMap, Method, Uri};

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
    create_impersonation_log_table(&db).await.unwrap();
    db
}

async fn add_user(db: &Database, name: &str, superuser: bool) -> User {
    User {
        id: 0,
        username: name.to_string(),
        email: format!("{name}@example.com"),
        password: String::new(),
        is_active: true,
        is_staff: superuser,
        is_superuser: superuser,
        date_joined: Utc::now(),
        last_login: None,
    }
    .save(db)
    .await
    .unwrap()
}

/// Builds a request whose session has the given user id (and impersonator, if any).
fn request_with_session(
    db: &Database,
    user_id: i64,
    impersonator: Option<i64>,
) -> (Request, Session) {
    let session = Session::new_empty();
    session.set(SESSION_USER_ID_KEY, user_id);
    session.set(SESSION_GENERATION_KEY, 0_i64);
    if let Some(imp) = impersonator {
        session.set(IMPERSONATOR_KEY, imp);
    }
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/impersonate/1"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(AppState::new().insert(db.clone()));
    (req, session)
}

#[tokio::test]
async fn superuser_can_start_and_stop_impersonation() {
    let db = setup().await;
    let admin = add_user(&db, "admin", true).await;
    let target = add_user(&db, "target", false).await;

    let (req, session) = request_with_session(&db, admin.id, None);
    let mut params = PathParams::new();
    params.insert("user_id", &target.id.to_string());
    start_impersonation_view(req, params).await.unwrap();

    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(target.id));
    assert_eq!(session.get::<i64>(IMPERSONATOR_KEY), Some(admin.id));

    // While impersonating: no nesting, and sensitive actions are refused.
    let (req, _) = request_with_session(&db, target.id, Some(admin.id));
    assert!(ensure_not_impersonating(&req).is_err());
    assert!(<NotImpersonating as FromRequest>::from_request(&req)
        .await
        .is_err());
    assert!(!impersonation_banner_html(&req).is_empty());

    // Stop restores the superuser.
    let (req, session) = request_with_session(&db, target.id, Some(admin.id));
    stop_impersonation_view(req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(admin.id));
    assert_eq!(session.get::<i64>(IMPERSONATOR_KEY), None);

    // Start and stop are both audit-logged.
    let mut conn = db.conn();
    let rows = conn
        .fetch_all("SELECT action FROM auth_impersonation_log ORDER BY id", &[])
        .await
        .unwrap();
    let actions: Vec<String> = rows
        .into_iter()
        .map(|r| r.try_string(0).unwrap().unwrap())
        .collect();
    assert_eq!(actions, vec!["start".to_string(), "stop".to_string()]);
}

#[tokio::test]
async fn non_superuser_cannot_impersonate() {
    let db = setup().await;
    let staff = add_user(&db, "staff", false).await;
    let target = add_user(&db, "other", false).await;
    let (req, session) = request_with_session(&db, staff.id, None);
    let mut params = PathParams::new();
    params.insert("user_id", &target.id.to_string());
    assert!(start_impersonation_view(req, params).await.is_err());
    assert_eq!(session.get::<i64>(IMPERSONATOR_KEY), None);
}

#[tokio::test]
async fn superusers_cannot_be_impersonated() {
    let db = setup().await;
    let admin = add_user(&db, "admin", true).await;
    let other_admin = add_user(&db, "admin2", true).await;
    let (req, session) = request_with_session(&db, admin.id, None);
    let mut params = PathParams::new();
    params.insert("user_id", &other_admin.id.to_string());
    assert!(start_impersonation_view(req, params).await.is_err());
    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(admin.id));
}

#[tokio::test]
async fn nested_impersonation_is_refused() {
    let db = setup().await;
    let admin = add_user(&db, "admin", true).await;
    let target = add_user(&db, "target", false).await;
    let third = add_user(&db, "third", false).await;
    let (req, _) = request_with_session(&db, target.id, Some(admin.id));
    let mut params = PathParams::new();
    params.insert("user_id", &third.id.to_string());
    assert!(start_impersonation_view(req, params).await.is_err());
}

#[test]
fn banner_is_empty_when_not_impersonating() {
    let req = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert!(impersonation_banner_html(&req).is_empty());
    assert_eq!(impersonator_id(&req), None);
}

#[tokio::test]
async fn request_impersonator_and_password_change_restriction() {
    let db = setup().await;
    let admin = add_user(&db, "admin", true).await;
    let mut target = add_user(&db, "target", false).await;

    let (req, _) = request_with_session(&db, target.id, Some(admin.id));
    let real = req.impersonator().await.unwrap().unwrap();
    assert_eq!(real.id, admin.id);

    let err = crate::session_mgmt::change_password(&req, &mut target, "NewStr0ngPass!99")
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::Session(_)));
}

#[test]
fn banner_includes_csrf_token_when_present() {
    let session = Session::new_empty();
    session.set(IMPERSONATOR_KEY, 1_i64);
    let mut extensions = Extensions::new();
    extensions.insert(session);
    extensions.insert(djangors_core::middleware::CsrfToken(
        "tok-abc-123".to_string(),
    ));
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions);
    let html = impersonation_banner_html(&req);
    assert!(html.contains("name=\"csrfmiddlewaretoken\""));
    assert!(html.contains("tok-abc-123"));
}

#[tokio::test]
async fn stop_clears_session_if_impersonator_lost_superuser() {
    let db = setup().await;
    let mut admin = add_user(&db, "admin", true).await;
    let target = add_user(&db, "target", false).await;

    admin.is_superuser = false;
    admin.update(&db).await.unwrap();

    let (req, session) = request_with_session(&db, target.id, Some(admin.id));
    let err = stop_impersonation_view(req, PathParams::new())
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), StatusCode::FORBIDDEN);
    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), None);
    assert_eq!(session.get::<i64>(IMPERSONATOR_KEY), None);
}
