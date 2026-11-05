use super::*;
// These were previously in scope implicitly: `tests` was a child module of a single monolithic
// lib.rs and picked them up through `use super::*`. After the crate was split into modules,
// lib.rs no longer imports them, so the test module declares them itself.
use bytes::Bytes;
use djangors_auth::User;
use djangors_auth::SESSION_USER_ID_KEY;
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_macros::Model as MacroModel;
use djangors_orm::Model as OrmModel;
use hyper::http::{Extensions, HeaderMap, Method, Uri};
use std::marker::PhantomData;

static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

static CROSS_PROCESS_LOCK: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
static EXCLUDED_TEST_FIELDS: &[&str] = &["name"];
static EXCLUDED_TEST_FIELDSETS: &[(&str, &[&str])] = &[("Credentials", &["name"])];

#[test]
fn excluded_admin_field_is_hidden_and_not_present_in_lists() {
    let admin = crate::model_admin::DefaultModelAdmin::<ModelD> {
        config: ModelAdminConfig {
            exclude: Some(EXCLUDED_TEST_FIELDS),
            search_fields: Some(EXCLUDED_TEST_FIELDS),
            list_filter: Some(EXCLUDED_TEST_FIELDS),
            ..Default::default()
        },
        _marker: PhantomData,
    };
    assert_eq!(admin.field_names(), vec!["id", "content"]);
    assert_eq!(admin.effective_columns(), vec!["id", "content"]);
    assert!(admin.search_fields().is_empty());
    assert!(admin.list_filter_fields().is_empty());

    let response = crate::forms::render_form(
        ModelD::meta(),
        &["id", "name", "content"],
        &std::collections::HashMap::from([
            ("id".to_string(), "1".to_string()),
            ("name".to_string(), "sensitive-value".to_string()),
            ("content".to_string(), "visible-value".to_string()),
        ]),
        &std::collections::HashMap::new(),
        false,
        &SiteBranding::default(),
        String::new(),
        true,
        false,
        Some(EXCLUDED_TEST_FIELDSETS),
        EXCLUDED_TEST_FIELDS,
        &[],
        &[],
        vec![],
    )
    .unwrap();
    let html = String::from_utf8_lossy(response.body());
    assert!(!html.contains("sensitive-value"));
    assert!(!html.contains("name=\"name\""));
    assert!(html.contains("visible-value"));
}

#[tokio::test]
async fn excluded_admin_field_is_ignored_during_update() {
    let db = djangors_db::Database::connect(
        &djangors_db::config::DatabaseConfig::new("sqlite::memory:").max_connections(1),
    )
    .await
    .unwrap();
    db.conn()
        .execute(
            "CREATE TABLE test_model_d (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, content TEXT NOT NULL)",
            &[],
        )
        .await
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO test_model_d (name, content) VALUES ('original', 'before')",
            &[],
        )
        .await
        .unwrap();
    let admin = crate::model_admin::DefaultModelAdmin::<ModelD> {
        config: ModelAdminConfig {
            exclude: Some(&["name"]),
            ..Default::default()
        },
        _marker: PhantomData,
    };
    admin
        .update_from_form(
            &db,
            1,
            &std::collections::HashMap::from([
                ("name".to_string(), "attacker-supplied".to_string()),
                ("content".to_string(), "after".to_string()),
            ]),
            None,
        )
        .await
        .unwrap()
        .unwrap();

    let row = ModelD::objects().get(&db).await.unwrap();
    assert_eq!(row.name, "original");
    assert_eq!(row.content, "after");
}

/// `DB_MUTEX` only serializes this crate's own tests against each other - it
/// says nothing about `cargo test --workspace` running a DIFFERENT crate's
/// test binary concurrently as a separate OS process, also racing to
/// DROP/CREATE the same `auth_user` table. Confirmed reproducible without
/// this: running djangors-admin and djangors-auth's suites concurrently
/// produced a real `relation "auth_user" already exists` (42P07) in one and
/// `relation "auth_user" does not exist` (42P01) in the other (task #61).
///
/// Acquires a cross-process Postgres advisory lock exactly once for this
/// entire test binary's lifetime (not per-test - `DB_MUTEX` itself already
/// releases and reacquires between tests, which would leave gaps for another
/// process to sneak in if this were also scoped per-test) and deliberately
/// leaks both the lock and its connection, since a session-level advisory
/// lock is released the moment its connection closes - the OS reclaiming
/// everything at process exit is the intended release mechanism here, not
/// an explicit `.release()` call.
async fn ensure_cross_process_lock_held() {
    if djangors_test::TestBackend::current() == djangors_test::TestBackend::Sqlite {
        return;
    }
    CROSS_PROCESS_LOCK
        .get_or_init(|| async {
            let db_url = std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors_test".into());
            let db =
                djangors_db::Database::connect(&djangors_db::config::DatabaseConfig::new(&db_url))
                    .await
                    .expect("connect for cross-process lock");
            let lock =
                djangors_test::acquire_cross_process_lock(&db, "djangors_test_auth_user_ddl")
                    .await
                    .expect("acquire cross-process lock");
            std::mem::forget(lock);
            std::mem::forget(db);
        })
        .await;
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_a")]
#[allow(dead_code)]
struct ModelA {
    #[djangors(primary_key, auto)]
    id: i64,
    name: String,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_b", ordering = "-title")]
#[allow(dead_code)]
struct ModelB {
    #[djangors(primary_key, auto)]
    id: i64,
    title: String,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_c")]
#[allow(dead_code)]
struct ModelC {
    #[djangors(primary_key, auto)]
    id: i64,
    parent: djangors_orm::ForeignKey<ModelA>,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_d")]
#[allow(dead_code)]
struct ModelD {
    #[djangors(primary_key, auto)]
    id: i64,
    name: String,
    content: String,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_e")]
#[allow(dead_code)]
struct ModelE {
    #[djangors(primary_key, auto)]
    id: i64,
    name: String,
    is_active: bool,
    is_staff: bool,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_f")]
#[allow(dead_code)]
struct ModelF {
    #[djangors(primary_key, auto)]
    id: i64,
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_g")]
#[allow(dead_code)]
struct ModelG {
    #[djangors(primary_key, auto)]
    id: i64,
    parent: djangors_orm::ForeignKey<ModelC>,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_h")]
#[allow(dead_code)]
struct ModelH {
    #[djangors(primary_key, auto)]
    id: i64,
    parent: djangors_orm::ForeignKey<ModelG>,
}

#[derive(MacroModel, Debug, Clone)]
#[djangors(app = "admin_test", table_name = "test_model_p")]
#[allow(dead_code)]
struct ModelP {
    #[djangors(primary_key, auto)]
    id: i64,
    #[djangors(foreign_key(on_delete = "protect"))]
    parent: djangors_orm::ForeignKey<ModelA>,
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_admin_index_endpoints() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_b", &[])
        .await;

    // Create auth_user
    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();

    // 1. Create a non-staff user
    let non_staff = User {
        id: 0,
        username: "non_staff".to_string(),
        email: "non_staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    // 2. Create a staff user
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    // Setup AdminSite
    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelB>();

    // Mount into a parent router, exactly like real usage
    let router = Router::new().mount("/admin", site.urls());

    // Test 1: GET /admin/ with no auth -> 401 Unauthorized
    let req_no_auth = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_no_auth = router.handle(req_no_auth).await;
    assert!(res_no_auth.is_err());
    assert_eq!(
        res_no_auth.unwrap_err().status_code(),
        StatusCode::UNAUTHORIZED
    );

    // Test 2: GET /admin/ authenticated as non-staff -> 403 Forbidden
    let session_non_staff = djangors_sessions::Session::new_empty();
    session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);
    let mut extensions_non_staff = Extensions::new();
    extensions_non_staff.insert(session_non_staff);
    let req_non_staff = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions_non_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_non_staff = router.handle(req_non_staff).await;
    assert!(res_non_staff.is_err());
    assert_eq!(
        res_non_staff.unwrap_err().status_code(),
        StatusCode::FORBIDDEN
    );

    // Test 3: GET /admin/ authenticated as staff -> 200 OK with correct HTML
    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);
    let mut extensions_staff = Extensions::new();
    extensions_staff.insert(session_staff);
    let req_staff = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_staff = router.handle(req_staff).await.unwrap();
    assert_eq!(res_staff.status(), StatusCode::OK);
    let body = String::from_utf8(res_staff.body().to_vec()).unwrap();
    assert!(body.contains("<li><a href=\"admin_test/modela/\">admin_test.ModelA</a></li>"));
    assert!(body.contains("<li><a href=\"admin_test/modelb/\">admin_test.ModelB</a></li>"));

    // Clean up
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_admin_changelist_endpoints() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_b", &[])
        .await;

    // Create auth_user
    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    // Create test_model_a
    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    // Create test_model_b (has ordering = "-title" meta)
    let create_model_b_sql = format!(
        "CREATE TABLE test_model_b (
                id {auto_pk},
                title TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_b_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();

    // Create users
    let non_staff = User {
        id: 0,
        username: "non_staff".to_string(),
        email: "non_staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    // Seed ModelA rows
    let _row1 = ModelA {
        id: 0,
        name: "Normal Row".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let _row2 = ModelA {
        id: 0,
        name: "<script>alert(1)</script>".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelB>();

    let router = Router::new().mount("/admin", site.urls());

    // Test 1: GET /admin/admin_test/modela/ as staff -> 200, checks headers & XSS escape
    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);
    let mut extensions_staff = Extensions::new();
    extensions_staff.insert(session_staff.clone());
    let req_list = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res_list = router.handle(req_list).await.unwrap();
    assert_eq!(res_list.status(), StatusCode::OK);
    let body = String::from_utf8(res_list.body().to_vec()).unwrap();

    // Assert header links
    assert!(body.contains("<a href=\"?o=id\">id</a>"));
    assert!(body.contains("<a href=\"?o=name\">name</a>"));

    // Assert row content and XSS escaping
    assert!(body.contains("Normal Row"));
    assert!(body.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;"));
    assert!(!body.contains("<script>"));
    // Assert changelist first cell contains change links
    assert!(body.contains("<a href=\"1/change/\">1</a>"));
    assert!(body.contains("<a href=\"2/change/\">2</a>"));

    // Test 2: Ordering tests
    // Seed rows for sorting
    let _ = db.conn().execute("DELETE FROM test_model_a", &[]).await;

    let _row_a = ModelA {
        id: 0,
        name: "Row A".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _row_b = ModelA {
        id: 0,
        name: "Row B".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    // GET with ascending order ?o=name
    let mut ext_staff = Extensions::new();
    ext_staff.insert(session_staff.clone());
    let req_asc = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/?o=name"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_asc = router.handle(req_asc).await.unwrap();
    let body_asc = String::from_utf8(res_asc.body().to_vec()).unwrap();
    let idx_a = body_asc.find("Row A").unwrap();
    let idx_b = body_asc.find("Row B").unwrap();
    assert!(idx_a < idx_b);

    // GET with descending order ?o=-name
    let mut ext_staff = Extensions::new();
    ext_staff.insert(session_staff.clone());
    let req_desc = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/?o=-name"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_desc = router.handle(req_desc).await.unwrap();
    let body_desc = String::from_utf8(res_desc.body().to_vec()).unwrap();
    let idx_a_desc = body_desc.find("Row A").unwrap();
    let idx_b_desc = body_desc.find("Row B").unwrap();
    assert!(idx_b_desc < idx_a_desc);

    // GET with invalid field ?o=nonsense -> 400
    let mut ext_staff = Extensions::new();
    ext_staff.insert(session_staff.clone());
    let req_invalid = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/?o=nonsense"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_invalid = router.handle(req_invalid).await;
    assert!(res_invalid.is_err());
    assert_eq!(
        res_invalid.unwrap_err().status_code(),
        StatusCode::BAD_REQUEST
    );

    // Test 3: Pagination tests
    // Seed 3 more rows to have 5 rows in total
    let _ = ModelA {
        id: 0,
        name: "Row C".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _ = ModelA {
        id: 0,
        name: "Row D".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _ = ModelA {
        id: 0,
        name: "Row E".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    // Direct trait call: admin.changelist(db, None, 2, 2)
    let admin_a = DefaultModelAdmin::<ModelA> {
        config: ModelAdminConfig::default(),
        _marker: PhantomData,
    };
    let page_data = admin_a
        .changelist(db, None, 2, 2, None, &[], None, None)
        .await
        .unwrap();
    assert_eq!(page_data.total, 5);
    assert_eq!(page_data.rows.len(), 2); // Row 3 and Row 4
    assert_eq!(page_data.rows[0][1], "Row C");
    assert_eq!(page_data.rows[1][1], "Row D");

    // HTTP assert that ?page=2 with the real constant (100) and few rows returns 200 with empty table body and Previous link
    let mut ext_staff = Extensions::new();
    ext_staff.insert(session_staff.clone());
    let req_page2 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/?page=2"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_page2 = router.handle(req_page2).await.unwrap();
    assert_eq!(res_page2.status(), StatusCode::OK);
    let body_page2 = String::from_utf8(res_page2.body().to_vec()).unwrap();
    assert!(body_page2.contains("<tbody></tbody>"));
    assert!(body_page2.contains("Previous"));

    // Test 4: Auth permissions
    // Unauthenticated -> 401
    let req_unauth = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_unauth = router.handle(req_unauth).await;
    assert!(res_unauth.is_err());
    assert_eq!(
        res_unauth.unwrap_err().status_code(),
        StatusCode::UNAUTHORIZED
    );

    // Non-staff -> 403
    let session_non_staff = djangors_sessions::Session::new_empty();
    session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);
    let mut ext_non_staff = Extensions::new();
    ext_non_staff.insert(session_non_staff);
    let req_non_staff = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_non_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_non_staff = router.handle(req_non_staff).await;
    assert!(res_non_staff.is_err());
    assert_eq!(
        res_non_staff.unwrap_err().status_code(),
        StatusCode::FORBIDDEN
    );

    // Unknown route as staff -> 404
    let mut ext_staff = Extensions::new();
    ext_staff.insert(session_staff.clone());
    let req_unknown = Request::new(
        Method::GET,
        Uri::from_static("/admin/nope/nope/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_unknown = router.handle(req_unknown).await;
    assert!(res_unknown.is_err());
    assert_eq!(
        res_unknown.unwrap_err().status_code(),
        StatusCode::NOT_FOUND
    );

    // Test 5: default order (no ?o=) honors the model's `ordering` meta -
    // ModelB declares ordering = "-title", so with no explicit sort param
    // "Beta" must come before "Alpha" even though "Alpha" was saved first.
    let _ = ModelB {
        id: 0,
        title: "Alpha".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _ = ModelB {
        id: 0,
        title: "Beta".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let mut ext_staff = Extensions::new();
    ext_staff.insert(session_staff.clone());
    let req_default_order = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelb/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext_staff)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_default_order = router.handle(req_default_order).await.unwrap();
    assert_eq!(res_default_order.status(), StatusCode::OK);
    let body_default_order = String::from_utf8(res_default_order.body().to_vec()).unwrap();
    let idx_beta = body_default_order.find("Beta").unwrap();
    let idx_alpha = body_default_order.find("Alpha").unwrap();
    assert!(idx_beta < idx_alpha);

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
    let _ = db.conn().execute("DROP TABLE test_model_a", &[]).await;
    let _ = db.conn().execute("DROP TABLE test_model_b", &[]).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_admin_change_form_endpoints() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let non_staff = User {
        id: 0,
        username: "non_staff".to_string(),
        email: "non_staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register::<ModelA>();
    let router = Router::new().mount("/admin", site.urls());

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    let session_non_staff = djangors_sessions::Session::new_empty();
    session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);

    // 1. GET add/ as staff -> 200, input per non-auto field (e.g. name)
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/add/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("name=\"name\""));
    assert!(!body.contains("name=\"id\"")); // id is auto, so omitted

    // 2. POST add/ valid data -> redirect + row exists in DB
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/add/"),
        headers,
        Bytes::from("name=New+Added+Row"),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/admin_test/modela/"
    );

    // Verify row exists
    let row = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(name = "New Added Row"))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    let new_pk = row.id;

    // 3. POST add/ with unparseable or validation errors (e.g. name is empty, which is required because it's not nullable) -> 200 re-rendered with error message
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/add/"),
        headers,
        Bytes::from("name="), // empty
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Field &#x27;name&#x27; is required."));

    // 4. GET change/{pk} -> 200 with prefilled value
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modela/{}/change/", new_pk)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("value=\"New Added Row\""));
    assert!(body.contains("id (readonly):"));

    // 5. POST change/{pk} valid -> row updated in DB
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::try_from(format!("/admin/admin_test/modela/{}/change/", new_pk)).unwrap(),
        headers,
        Bytes::from("name=Updated+Row+Name"),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);

    let row_updated = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = new_pk))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(row_updated.name, "Updated Row Name");

    // 6. GET change/{pk} for nonexistent pk -> 404
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/99999/change/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::NOT_FOUND);

    // 7. Unauthenticated GET add/ -> 401
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/add/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::UNAUTHORIZED);

    // 8. Non-staff GET add/ -> 403
    let mut ext = Extensions::new();
    ext.insert(session_non_staff);
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/add/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::FORBIDDEN);

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
    let _ = db.conn().execute("DROP TABLE test_model_a", &[]).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_admin_delete_endpoints() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_model_c_sql = format!(
        "CREATE TABLE test_model_c (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_c_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let non_staff = User {
        id: 0,
        username: "non_staff".to_string(),
        email: "non_staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelC>();
    let router = Router::new().mount("/admin", site.urls());

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    let session_non_staff = djangors_sessions::Session::new_empty();
    session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);

    // Insert test data
    let parent_pk = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![(
            "name",
            djangors_orm::expr::Value::Text("Parent Object".to_string()),
        )],
    )
    .await
    .unwrap();

    // Insert child referencing parent
    let child_pk = djangors_orm::queryset::QuerySet::<ModelC>::insert_raw(
        db,
        vec![("parent", djangors_orm::expr::Value::I64(parent_pk))],
    )
    .await
    .unwrap();

    // a. GET delete confirm page for an existing pk as staff -> 200, contains field values and related ModelC warning showing count 1
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", parent_pk)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Parent Object"));
    assert!(body.contains("ModelC") || body.contains("test_model_c"));
    assert!(body.contains("1")); // shows count of 1

    // b. GET delete confirm page for a nonexistent pk -> 404
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/99999/delete/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::NOT_FOUND);

    // c. Unauthenticated GET -> 401. Non-staff GET -> 403.
    let req = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", parent_pk)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::UNAUTHORIZED);

    let mut ext = Extensions::new();
    ext.insert(session_non_staff);
    let req = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", parent_pk)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::FORBIDDEN);

    // d. POST delete for an existing pk
    // First delete child
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::POST,
        Uri::try_from(format!("/admin/admin_test/modelc/{}/delete/", child_pk)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/admin_test/modelc/"
    );

    // Verify child is gone from DB
    let child_check = djangors_orm::queryset::QuerySet::<ModelC>::new()
        .filter(djangors_orm::q!(id = child_pk))
        .unwrap()
        .get(db)
        .await;
    assert!(child_check.is_err());

    // e. POST delete for a nonexistent pk -> 404
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modelc/99999/delete/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::NOT_FOUND);

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
    let _ = db.conn().execute("DROP TABLE test_model_a", &[]).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_list_display_and_search() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    // 1. register_with with a list_display subset that excludes the pk field
    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        list_display: Some(&["content", "name"]),
        search_fields: None,
        ..Default::default()
    });

    let pk_d = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Alice".to_string())),
            (
                "content",
                djangors_orm::expr::Value::Text("Hello world".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let router = Router::new().mount("/admin", site.urls());
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("content"));
    assert!(body.contains("name"));
    assert!(!body.contains("<th><a href=\"?o=id\">id</a></th>"));
    assert!(!body.contains("<th><a href=\"?o=-id\">id</a></th>"));
    assert!(body.contains(&format!("<a href=\"{}/change/\">Hello world</a>", pk_d)));

    // 2. register_with with a list_display naming a field that doesn't exist on the model -> panics
    let site2 = AdminSite::new();
    let res2 = std::panic::catch_unwind(|| {
        site2.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["nonexistent"]),
            search_fields: None,
            ..Default::default()
        });
    });
    assert!(res2.is_err());

    // 3. register_with with search_fields naming a non-text field -> panics
    let site3 = AdminSite::new();
    let res3 = std::panic::catch_unwind(|| {
        site3.register_with::<ModelD>(ModelAdminConfig {
            list_display: None,
            search_fields: Some(&["id"]),
            ..Default::default()
        });
    });
    assert!(res3.is_err());

    // Clean up rows for search tests
    let _ = db.conn().execute("DELETE FROM test_model_d", &[]).await;

    // Insert 3 rows
    let _ = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Apple".to_string())),
            (
                "content",
                djangors_orm::expr::Value::Text("First".to_string()),
            ),
        ],
    )
    .await
    .unwrap();
    let _ = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("Banana".to_string()),
            ),
            (
                "content",
                djangors_orm::expr::Value::Text("Second".to_string()),
            ),
        ],
    )
    .await
    .unwrap();
    let _ = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("Apricot".to_string()),
            ),
            (
                "content",
                djangors_orm::expr::Value::Text("Third".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    // 4. search_fields configured, ?q= matching substring (case-insensitive)
    let site_search = AdminSite::new();
    site_search.register_with::<ModelD>(ModelAdminConfig {
        list_display: None,
        search_fields: Some(&["name"]),
        ..Default::default()
    });
    let router_search = Router::new().mount("/admin", site_search.urls());

    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/?q=ap"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router_search.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Apple"));
    assert!(body.contains("Apricot"));
    assert!(!body.contains("Banana"));
    assert!(body.contains("Total: 2."));

    // 5. search_fields configured but ?q= omitted
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router_search.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Apple"));
    assert!(body.contains("Banana"));
    assert!(body.contains("Apricot"));
    assert!(body.contains("Total: 3."));

    // 6. No search_fields configured but ?q=something passed -> q is ignored, all show
    let site_nosearch = AdminSite::new();
    site_nosearch.register_with::<ModelD>(ModelAdminConfig::default());
    let router_nosearch = Router::new().mount("/admin", site_nosearch.urls());

    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/?q=Banana"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router_nosearch.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Apple"));
    assert!(body.contains("Banana"));
    assert!(body.contains("Apricot"));
    assert!(body.contains("Total: 3."));

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_list_filter() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_e", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_e_sql = format!(
        "CREATE TABLE test_model_e (
                id {auto_pk},
                name TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL
            )"
    );
    db.conn().execute(&create_model_e_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    // 1. register_with with list_filter: Some(&["is_active"]) on a model with a Boolean field -> registers successfully
    let site = AdminSite::new();
    site.register_with::<ModelE>(ModelAdminConfig {
        list_filter: Some(&["is_active"]),
        ..Default::default()
    });

    // Insert some rows to verify matching logic later
    let _ = djangors_orm::queryset::QuerySet::<ModelE>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Alice".to_string())),
            ("is_active", djangors_orm::expr::Value::Bool(true)),
            ("is_staff", djangors_orm::expr::Value::Bool(false)),
        ],
    )
    .await
    .unwrap();

    let _ = djangors_orm::queryset::QuerySet::<ModelE>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Bob".to_string())),
            ("is_active", djangors_orm::expr::Value::Bool(true)),
            ("is_staff", djangors_orm::expr::Value::Bool(true)),
        ],
    )
    .await
    .unwrap();

    let _ = djangors_orm::queryset::QuerySet::<ModelE>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("Charlie".to_string()),
            ),
            ("is_active", djangors_orm::expr::Value::Bool(false)),
            ("is_staff", djangors_orm::expr::Value::Bool(false)),
        ],
    )
    .await
    .unwrap();

    let router = Router::new().mount("/admin", site.urls());
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modele/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    // contains the filter UI (All/Yes/No links for that field)
    assert!(body.contains("Filter by is_active:"));
    assert!(body.contains("<a href=\"\">All</a>"));
    assert!(body.contains("<a href=\"?is_active=true\">Yes</a>"));
    assert!(body.contains("<a href=\"?is_active=false\">No</a>"));

    // 2. register_with with list_filter naming a field that doesn't exist -> panics
    let site2 = AdminSite::new();
    let res2 = std::panic::catch_unwind(|| {
        site2.register_with::<ModelE>(ModelAdminConfig {
            list_filter: Some(&["nonexistent"]),
            ..Default::default()
        });
    });
    assert!(res2.is_err());

    // 3. register_with with list_filter naming a real but non-Boolean field -> panics
    let site3 = AdminSite::new();
    let res3 = std::panic::catch_unwind(|| {
        site3.register_with::<ModelE>(ModelAdminConfig {
            list_filter: Some(&["name"]),
            ..Default::default()
        });
    });
    assert!(res3.is_err());

    // 4. GET with ?is_active=true -> only the matching rows appear, total count is correct
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req_filter = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modele/?is_active=true"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_filter = router.handle(req_filter).await.unwrap();
    assert_eq!(res_filter.status(), StatusCode::OK);
    let body_filter = String::from_utf8(res_filter.body().to_vec()).unwrap();
    assert!(body_filter.contains("Alice"));
    assert!(body_filter.contains("Bob"));
    assert!(!body_filter.contains("Charlie"));
    assert!(body_filter.contains("Total: 2."));

    // 5. Same setup, GET with ?is_active=true&q=<term> (both active, assuming search_fields configured)
    let site_both = AdminSite::new();
    site_both.register_with::<ModelE>(ModelAdminConfig {
        search_fields: Some(&["name"]),
        list_filter: Some(&["is_active"]),
        ..Default::default()
    });
    let router_both = Router::new().mount("/admin", site_both.urls());

    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req_both = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modele/?is_active=true&q=Bob"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_both = router_both.handle(req_both).await.unwrap();
    assert_eq!(res_both.status(), StatusCode::OK);
    let body_both = String::from_utf8(res_both.body().to_vec()).unwrap();
    assert!(!body_both.contains("Alice"));
    assert!(body_both.contains("Bob"));
    assert!(!body_both.contains("Charlie"));
    assert!(body_both.contains("Total: 1."));

    // 6. GET with a query parameter matching a field name that is real on the model but NOT in list_filter -> ignored entirely
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req_ignored = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modele/?is_staff=true"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_ignored = router.handle(req_ignored).await.unwrap();
    assert_eq!(res_ignored.status(), StatusCode::OK);
    let body_ignored = String::from_utf8(res_ignored.body().to_vec()).unwrap();
    assert!(body_ignored.contains("Alice"));
    assert!(body_ignored.contains("Bob"));
    assert!(body_ignored.contains("Charlie"));
    assert!(body_ignored.contains("Total: 3."));

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_e", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_part6_3_bulk_delete() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let non_staff = User {
        id: 0,
        username: "non_staff".to_string(),
        email: "non_staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    let session_non_staff = djangors_sessions::Session::new_empty();
    session_non_staff.set(SESSION_USER_ID_KEY, non_staff.id);

    let site = AdminSite::new();
    site.register::<ModelA>();
    let router = Router::new().mount("/admin", site.urls());

    // Insert 3 rows
    let pk1 = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![("name", djangors_orm::expr::Value::Text("First".to_string()))],
    )
    .await
    .unwrap();

    let pk2 = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![(
            "name",
            djangors_orm::expr::Value::Text("Second".to_string()),
        )],
    )
    .await
    .unwrap();

    let pk3 = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![("name", djangors_orm::expr::Value::Text("Third".to_string()))],
    )
    .await
    .unwrap();

    // 1. Insert 3 rows. POST to bulk-delete/ with selected=<pk1>&selected=<pk2> (no confirm) ->
    //    200, response body lists both selected objects' display values, does NOT mention the third
    //    (unselected) row, and both rows still exist in the DB afterward.
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers.clone(),
        Bytes::from(format!("selected={}&selected={}", pk1, pk2)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("First"));
    assert!(body.contains("Second"));
    assert!(!body.contains("Third"));

    // verify rows still exist
    let check1 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk1))
        .unwrap()
        .get(db)
        .await;
    let check2 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk2))
        .unwrap()
        .get(db)
        .await;
    let check3 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk3))
        .unwrap()
        .get(db)
        .await;
    assert!(check1.is_ok());
    assert!(check2.is_ok());
    assert!(check3.is_ok());

    // 2. POST the same selected=<pk1>&selected=<pk2> again, this time WITH confirm=1 ->
    //    302 redirect to the changelist, both rows actually gone from the DB, the third (unselected) row still present.
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers.clone(),
        Bytes::from(format!("selected={}&selected={}&confirm=1", pk1, pk2)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/admin_test/modela/"
    );

    let check1 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk1))
        .unwrap()
        .get(db)
        .await;
    let check2 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk2))
        .unwrap()
        .get(db)
        .await;
    let check3 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk3))
        .unwrap()
        .get(db)
        .await;
    assert!(check1.is_err());
    assert!(check2.is_err());
    assert!(check3.is_ok());

    // 3. POST with no selected values at all (empty selection) -> redirects without error, nothing deleted.
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers.clone(),
        Bytes::from(""),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/admin_test/modela/"
    );

    let check3 = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(id = pk3))
        .unwrap()
        .get(db)
        .await;
    assert!(check3.is_ok());

    // 4. POST with a selected value that isn't a valid integer -> 400 Bad Request.
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers.clone(),
        Bytes::from("selected=abc"),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res = router.handle(req).await;
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().status_code(), StatusCode::BAD_REQUEST);

    // 5. Unauthenticated / non-staff POST -> 401 / 403.
    // Unauthenticated:
    let req_unauth = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers.clone(),
        Bytes::from("selected=1"),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_unauth = router.handle(req_unauth).await;
    assert!(res_unauth.is_err());
    assert_eq!(
        res_unauth.unwrap_err().status_code(),
        StatusCode::UNAUTHORIZED
    );

    // Non-staff:
    let mut ext = Extensions::new();
    ext.insert(session_non_staff.clone());
    let req_nonstaff = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers,
        Bytes::from("selected=1"),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_nonstaff = router.handle(req_nonstaff).await;
    assert!(res_nonstaff.is_err());
    assert_eq!(
        res_nonstaff.unwrap_err().status_code(),
        StatusCode::FORBIDDEN
    );

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_part6_4_date_hierarchy() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_f", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_f_sql = format!(
        "CREATE TABLE test_model_f (
                id {auto_pk},
                name TEXT NOT NULL,
                created_at {ts_type} NOT NULL
            )"
    );
    db.conn().execute(&create_model_f_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    use chrono::{TimeZone, Utc};

    let now = Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    let dt1 = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).single().unwrap();
    let dt2 = Utc.with_ymd_and_hms(2026, 3, 10, 0, 0, 0).single().unwrap();
    let dt3 = Utc.with_ymd_and_hms(2026, 3, 20, 0, 0, 0).single().unwrap();
    let dt4 = Utc.with_ymd_and_hms(2025, 11, 5, 0, 0, 0).single().unwrap();

    let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Row1".to_string())),
            ("created_at", djangors_orm::expr::Value::DateTime(dt1)),
        ],
    )
    .await
    .unwrap();

    let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Row2".to_string())),
            ("created_at", djangors_orm::expr::Value::DateTime(dt2)),
        ],
    )
    .await
    .unwrap();

    let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Row3".to_string())),
            ("created_at", djangors_orm::expr::Value::DateTime(dt3)),
        ],
    )
    .await
    .unwrap();

    let _ = djangors_orm::queryset::QuerySet::<ModelF>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Row4".to_string())),
            ("created_at", djangors_orm::expr::Value::DateTime(dt4)),
        ],
    )
    .await
    .unwrap();

    // Register site
    let site = AdminSite::new();
    site.register_with::<ModelF>(ModelAdminConfig {
        date_hierarchy: Some("created_at"),
        ..Default::default()
    });

    let router = Router::new().mount("/admin", site.urls());

    // Test 1: GET the changelist with no year/month/day -> 200
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelf/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res1 = router.handle(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = String::from_utf8(res1.body().to_vec()).unwrap();
    assert!(body1.contains("2025"));
    assert!(body1.contains("2026"));
    assert!(!body1.contains("January"));
    assert!(!body1.contains("March"));

    // Test 2: GET with ?year=2026 -> 200, only contains Row1, Row2, Row3
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req2 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelf/?year=2026"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res2 = router.handle(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let body2 = String::from_utf8(res2.body().to_vec()).unwrap();
    assert!(body2.contains("Row1"));
    assert!(body2.contains("Row2"));
    assert!(body2.contains("Row3"));
    assert!(!body2.contains("Row4"));
    assert!(body2.contains("January"));
    assert!(body2.contains("March"));
    assert!(!body2.contains("February"));

    // Test 3: GET with ?year=2026&month=3 -> 200, contains Row2 and Row3
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req3 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelf/?year=2026&month=3"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res3 = router.handle(req3).await.unwrap();
    assert_eq!(res3.status(), StatusCode::OK);
    let body3 = String::from_utf8(res3.body().to_vec()).unwrap();
    assert!(!body3.contains("Row1"));
    assert!(body3.contains("Row2"));
    assert!(body3.contains("Row3"));
    assert!(body3.contains(">10<"));
    assert!(body3.contains(">20<"));

    // Test 4: GET with ?year=2026&month=3&day=10 -> 200, contains only Row2
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req4 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelf/?year=2026&month=3&day=10"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res4 = router.handle(req4).await.unwrap();
    assert_eq!(res4.status(), StatusCode::OK);
    let body4 = String::from_utf8(res4.body().to_vec()).unwrap();
    assert!(!body4.contains("Row1"));
    assert!(body4.contains("Row2"));
    assert!(!body4.contains("Row3"));

    // Test 5: GET with ?year=2026&o=name -> 200, sort header contains year=2026
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req5 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelf/?year=2026&o=name"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res5 = router.handle(req5).await.unwrap();
    assert_eq!(res5.status(), StatusCode::OK);
    let body5 = String::from_utf8(res5.body().to_vec()).unwrap();
    assert!(body5.contains("year=2026"));

    // Test 6: GET with invalid ?month=13 -> 400 Bad Request
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req6 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelf/?year=2026&month=13"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res6 = router.handle(req6).await;
    assert!(res6.is_err());
    assert_eq!(res6.unwrap_err().status_code(), StatusCode::BAD_REQUEST);

    // Test 7: Registering a model with date_hierarchy: Some("name") (non-DateTime) -> panics
    let site_panic = AdminSite::new();
    let res7 = std::panic::catch_unwind(|| {
        site_panic.register_with::<ModelD>(ModelAdminConfig {
            date_hierarchy: Some("name"),
            ..Default::default()
        });
    });
    assert!(res7.is_err());

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_f", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn test_phase5_part6_5_list_editable() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_e", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let create_model_e_sql = format!(
        "CREATE TABLE test_model_e (
                id {auto_pk},
                name TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL
            )"
    );
    db.conn().execute(&create_model_e_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    use chrono::Utc;
    let now = Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    let pk1 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Row1".to_string())),
            (
                "content",
                djangors_orm::expr::Value::Text("Content1".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let pk2 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Row2".to_string())),
            (
                "content",
                djangors_orm::expr::Value::Text("Content2".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    // 1. Register ModelD and check GET changelist -> 200, input field, save button
    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        list_display: Some(&["name", "content"]),
        list_editable: Some(&["content"]),
        ..Default::default()
    });

    let router = Router::new().mount("/admin", site.urls());

    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res1 = router.handle(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let body1 = String::from_utf8(res1.body().to_vec()).unwrap();
    assert!(body1.contains(&format!("name=\"edit-{}-content\"", pk1)));
    assert!(!body1.contains(&format!("name=\"edit-{}-name\"", pk1)));
    assert!(body1.contains("formaction=\"save-changelist/\""));

    // 2. Registration-validation panics
    // (a) list_editable naming a field not in list_display
    let site_panic = AdminSite::new();
    let res_a = std::panic::catch_unwind(|| {
        site_panic.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["name"]),
            list_editable: Some(&["content"]),
            ..Default::default()
        });
    });
    assert!(res_a.is_err());

    // (b) list_editable naming list_display's first column
    let res_b = std::panic::catch_unwind(|| {
        site_panic.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["name", "content"]),
            list_editable: Some(&["name"]),
            ..Default::default()
        });
    });
    assert!(res_b.is_err());

    // (c) list_editable naming a Boolean field
    let res_c = std::panic::catch_unwind(|| {
        site_panic.register_with::<ModelE>(ModelAdminConfig {
            list_display: Some(&["name", "is_active"]),
            list_editable: Some(&["is_active"]),
            ..Default::default()
        });
    });
    assert!(res_c.is_err());

    // (d) list_editable naming a nonexistent field
    let res_d = std::panic::catch_unwind(|| {
        site_panic.register_with::<ModelD>(ModelAdminConfig {
            list_display: Some(&["name", "content"]),
            list_editable: Some(&["nonexistent"]),
            ..Default::default()
        });
    });
    assert!(res_d.is_err());

    // 3. POST save-changelist/ with edit-<pk1>-content=Updated1 for one row -> 302 redirect
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );

    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req3 = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers.clone(),
        Bytes::from(format!("edit-{}-content=Updated1", pk1)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res3 = router.handle(req3).await.unwrap();
    assert_eq!(res3.status(), StatusCode::FOUND);

    // Verify updated in DB
    let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
        .filter(djangors_orm::q!(id = pk1))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(check1.content, "Updated1");

    // 4. POST with edits for TWO different pks in the same request (edit-<pk1>-content=A and edit-<pk2>-content=B) -> 302 redirect, BOTH rows updated correctly
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req4 = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers.clone(),
        Bytes::from(format!("edit-{}-content=A&edit-{}-content=B", pk1, pk2)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res4 = router.handle(req4).await.unwrap();
    assert_eq!(res4.status(), StatusCode::FOUND);

    let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
        .filter(djangors_orm::q!(id = pk1))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(check1.content, "A");
    let check2 = djangors_orm::queryset::QuerySet::<ModelD>::new()
        .filter(djangors_orm::q!(id = pk2))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(check2.content, "B");

    // 5. POST with edit-<pk>-content= (empty string) where content is non-nullable -> 200 (not a redirect), response body mentions content in error message, and DB row's content is UNCHANGED
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req5 = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers.clone(),
        Bytes::from(format!("edit-{}-content=", pk1)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res5 = router.handle(req5).await.unwrap();
    assert_eq!(res5.status(), StatusCode::OK);
    let body5 = String::from_utf8(res5.body().to_vec()).unwrap();
    assert!(body5.contains("content"));
    // content is still "A"
    let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
        .filter(djangors_orm::q!(id = pk1))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(check1.content, "A");

    // 6. POST with an edit-<pk>-name=Malicious key, where name is NOT in list_editable -> redirect, DB row's name is UNCHANGED
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req6 = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers.clone(),
        Bytes::from(format!("edit-{}-name=Malicious", pk1)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res6 = router.handle(req6).await.unwrap();
    assert_eq!(res6.status(), StatusCode::FOUND);
    let check1 = djangors_orm::queryset::QuerySet::<ModelD>::new()
        .filter(djangors_orm::q!(id = pk1))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    assert_eq!(check1.name, "Row1");

    // 7. POST with a non-numeric pk in an edit- key (e.g. edit-abc-content=X) -> 400 Bad Request
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req7 = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers.clone(),
        Bytes::from("edit-abc-content=X"),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res7 = router.handle(req7).await;
    assert!(res7.is_err());
    assert_eq!(res7.unwrap_err().status_code(), StatusCode::BAD_REQUEST);

    // 8. Unauthenticated / non-staff POST to save-changelist/ -> 401 / 403
    // Unauthenticated:
    let req8_unauth = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers.clone(),
        Bytes::from(format!("edit-{}-content=Unauth", pk1)),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res8_unauth = router.handle(req8_unauth).await;
    assert!(res8_unauth.is_err());
    assert!(matches!(
        res8_unauth.unwrap_err().status_code(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_e", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn test_phase5_part6_6_csv_export() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop/create tables
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    use chrono::Utc;
    let now = Utc::now();
    let staff = User {
        id: 0,
        username: "staff".to_string(),
        email: "staff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    // Insert 3 ModelD rows
    let _pk1 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Apple".to_string())),
            (
                "content",
                djangors_orm::expr::Value::Text("Fruit".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let _pk2 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("Banana".to_string()),
            ),
            (
                "content",
                djangors_orm::expr::Value::Text("Yellow".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let _pk3 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("Cherry".to_string()),
            ),
            (
                "content",
                djangors_orm::expr::Value::Text("Red".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    // 2. Register with default config
    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        search_fields: Some(&["name", "content"]),
        ..Default::default()
    });

    let router = Router::new().mount("/admin", site.urls());

    // GET export-csv/ with no params
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/export-csv/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res1 = router.handle(req1).await.unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let content_type = res1
        .headers()
        .get("Content-Type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_type.starts_with("text/csv"));
    let content_disp = res1
        .headers()
        .get("Content-Disposition")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(content_disp.contains("attachment; filename=\"modeld.csv\""));

    let body1 = String::from_utf8(res1.body().to_vec()).unwrap();
    let lines: Vec<&str> = body1.split("\r\n").filter(|s| !s.is_empty()).collect();
    assert_eq!(lines.len(), 4); // header + 3 rows
    assert_eq!(lines[0], "id,name,content");
    assert!(body1.contains("Apple"));
    assert!(body1.contains("Banana"));
    assert!(body1.contains("Cherry"));

    // 3. GET export-csv/?q=Banana
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req2 = Request::new(
        Method::GET,
        "/admin/admin_test/modeld/export-csv/?q=Banana"
            .parse::<Uri>()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res2 = router.handle(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let body2 = String::from_utf8(res2.body().to_vec()).unwrap();
    let lines2: Vec<&str> = body2.split("\r\n").filter(|s| !s.is_empty()).collect();
    assert_eq!(lines2.len(), 2); // header + 1 row
    assert!(!body2.contains("Apple"));
    assert!(body2.contains("Banana"));
    assert!(!body2.contains("Cherry"));

    // 4. Insert row with comma:
    let _pk4 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("Date, Fruit".to_string()),
            ),
            (
                "content",
                djangors_orm::expr::Value::Text("Sweet, sticky".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req3 = Request::new(
        Method::GET,
        "/admin/admin_test/modeld/export-csv/?q=sticky"
            .parse::<Uri>()
            .unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res3 = router.handle(req3).await.unwrap();
    assert_eq!(res3.status(), StatusCode::OK);
    let body3 = String::from_utf8(res3.body().to_vec()).unwrap();
    assert!(body3.contains("\"Date, Fruit\""));
    assert!(body3.contains("\"Sweet, sticky\""));

    // 5. Unauthenticated
    let req4 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/export-csv/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res4 = router.handle(req4).await;
    assert!(res4.is_err());
    assert!(matches!(
        res4.unwrap_err().status_code(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ));

    // Clean up
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn test_phase5_computed_columns() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username TEXT NOT NULL UNIQUE,
                email TEXT NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_computed".to_string(),
        email: "staff_computed@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    fn format_combo(values: &[(&'static str, djangors_orm::expr::Value)]) -> String {
        let name = values
            .iter()
            .find(|(n, _)| *n == "name")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        let content = values
            .iter()
            .find(|(n, _)| *n == "content")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        format!("{} - {}", name, content)
    }

    let _pk = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("Alice".to_string())),
            (
                "content",
                djangors_orm::expr::Value::Text("Hello".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        list_display: Some(&["name", "combo"]),
        computed_columns: Some(&[("combo", format_combo)]),
        ..Default::default()
    });

    let router = Router::new().mount("/admin", site.urls());
    let mut ext = Extensions::new();
    ext.insert(session_staff);
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Alice - Hello"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_bulk_action_no_confirm() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_action".to_string(),
        email: "staff_action@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    fn touch_action<'a>(
        _db: &'a djangors_db::Database,
        _pks: &'a [i64],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>>
    {
        Box::pin(async move { Ok(()) })
    }

    let pk = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![(
            "name",
            djangors_orm::expr::Value::Text("ActionTarget".to_string()),
        )],
    )
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register_with::<ModelA>(ModelAdminConfig {
        actions: Some(&[AdminAction {
            name: "custom_touch",
            label: "Touch",
            requires_confirm: false,
            handler: touch_action,
        }]),
        ..Default::default()
    });

    let router = Router::new().mount("/admin", site.urls());

    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );

    let mut ext = Extensions::new();
    ext.insert(session_staff);
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-action/"),
        headers,
        Bytes::from(format!("action=custom_touch&selected={}", pk)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/admin_test/modela/"
    );

    let logs = djangors_orm::queryset::QuerySet::<LogEntry>::new()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].action_flag, ACTION_CHANGE);
    assert_eq!(logs[0].change_message, "Touch");

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_bulk_action_with_confirm() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_confirm".to_string(),
        email: "staff_confirm@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);

    fn confirm_action<'a>(
        _db: &'a djangors_db::Database,
        _pks: &'a [i64],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DjangorsError>> + Send + 'a>>
    {
        Box::pin(async move { Ok(()) })
    }

    let pk = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![(
            "name",
            djangors_orm::expr::Value::Text("ConfirmTarget".to_string()),
        )],
    )
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register_with::<ModelA>(ModelAdminConfig {
        actions: Some(&[AdminAction {
            name: "needs_confirmation",
            label: "Needs Confirm",
            requires_confirm: true,
            handler: confirm_action,
        }]),
        ..Default::default()
    });

    let router = Router::new().mount("/admin", site.urls());

    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );

    // Step 1: POST without confirm=1 -> should show confirm page
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-action/"),
        headers.clone(),
        Bytes::from(format!("action=needs_confirmation&selected={}", pk)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Needs Confirm"));
    assert!(body.contains("ConfirmTarget"));
    assert!(body.contains("confirm"));

    // Step 2: POST with confirm=1 -> should execute and redirect
    let mut ext = Extensions::new();
    ext.insert(session_staff);
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-action/"),
        headers,
        Bytes::from(format!(
            "action=needs_confirmation&selected={}&confirm=1",
            pk
        )),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);
    assert_eq!(
        res.headers().get("location").unwrap().to_str().unwrap(),
        "/admin_test/modela/"
    );

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_admin_permissions_enforcement() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop tables
    let drop_tables = [
        "auth_user_permissions",
        "auth_group_permissions",
        "auth_user_groups",
        "auth_group",
        "auth_permission",
        "auth_user",
        "test_model_a",
        "test_model_b",
    ];
    for table in drop_tables {
        let _ = db
            .conn()
            .execute(&format!("DROP TABLE IF EXISTS {}", table), &[])
            .await;
    }

    // Create tables
    db.conn()
        .execute(
            &format!(
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
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_permission (
                id {auto_pk},
                codename VARCHAR(255) NOT NULL UNIQUE,
                name VARCHAR(255) NOT NULL
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_group (
                id {auto_pk},
                name VARCHAR(150) NOT NULL UNIQUE
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user_groups (
                id {auto_pk},
                \"user\" BIGINT NOT NULL,
                \"group\" BIGINT NOT NULL
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_group_permissions (
                id {auto_pk},
                \"group\" BIGINT NOT NULL,
                permission BIGINT NOT NULL
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user_permissions (
                id {auto_pk},
                \"user\" BIGINT NOT NULL,
                permission BIGINT NOT NULL
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_model_a (
                id {auto_pk},
                name TEXT NOT NULL
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE test_model_b (
                id {auto_pk},
                title TEXT NOT NULL
            )"
            ),
            &[],
        )
        .await
        .unwrap();

    let now = chrono::Utc::now();

    // Create staff, non-superuser user
    let staff_user = User {
        id: 0,
        username: "staff_user".to_string(),
        email: "staff_user@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    // Setup AdminSite
    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelB>();
    let router = Router::new().mount("/admin", site.urls());

    // Create session
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff_user.id);
    let mut ext = Extensions::new();
    ext.insert(session.clone());

    // Test 1: A staff, non-superuser user with no permissions at all -> GET changelist for ModelA -> 403 Forbidden
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res1 = router.handle(req1).await;
    assert!(res1.is_err());
    assert_eq!(res1.unwrap_err().status_code(), StatusCode::FORBIDDEN);

    // Test 2: The same user, after being granted the `view` permission for that specific model -> GET changelist -> 200
    let codename_view_a = "admin_test.view_modela";
    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    db.conn()
        .execute(
            &format!("INSERT INTO auth_permission (codename, name) VALUES ({p1}, {p2})"),
            &[
                djangors_db::BindValue::Text(codename_view_a.into()),
                djangors_db::BindValue::Text("Can view modela".into()),
            ],
        )
        .await
        .unwrap();

    let perm_id: i64 = db
        .conn()
        .fetch_one(
            "SELECT id FROM auth_permission WHERE codename = $1",
            &[djangors_db::BindValue::Text(codename_view_a.into())],
        )
        .await
        .unwrap()
        .try_i64(0)
        .unwrap()
        .unwrap();

    // Link user to permission
    db.conn()
        .execute(
            &format!(
                "INSERT INTO auth_user_permissions (\"user\", permission) VALUES ({p1}, {p2})"
            ),
            &[
                djangors_db::BindValue::I64(staff_user.id),
                djangors_db::BindValue::I64(perm_id),
            ],
        )
        .await
        .unwrap();

    let req2 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res2 = router.handle(req2).await.unwrap();
    assert_eq!(res2.status(), StatusCode::OK);

    // Test 3: The same permissioned-for-view-only user -> POST to that model's add/ route -> 403
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req3 = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/add/"),
        headers,
        Bytes::from("name=TestingAdd"),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res3 = router.handle(req3).await;
    assert!(res3.is_err());
    assert_eq!(res3.unwrap_err().status_code(), StatusCode::FORBIDDEN);

    // Clean up direct permission for next tests
    db.conn()
        .execute("DELETE FROM auth_user_permissions", &[])
        .await
        .unwrap();

    // Test 4: A staff, non-superuser user granted view via group membership -> GET changelist -> 200
    db.conn()
        .execute(
            &format!("INSERT INTO auth_group (name) VALUES ({p1})"),
            &[djangors_db::BindValue::Text("Editors".into())],
        )
        .await
        .unwrap();
    let group_id: i64 = db
        .conn()
        .fetch_one("SELECT id FROM auth_group WHERE name = 'Editors'", &[])
        .await
        .unwrap()
        .try_i64(0)
        .unwrap()
        .unwrap();

    // Link group to permission
    db.conn()
        .execute(
            &format!(
                "INSERT INTO auth_group_permissions (\"group\", permission) VALUES ({p1}, {p2})"
            ),
            &[
                djangors_db::BindValue::I64(group_id),
                djangors_db::BindValue::I64(perm_id),
            ],
        )
        .await
        .unwrap();

    // Link user to group
    db.conn()
        .execute(
            &format!("INSERT INTO auth_user_groups (\"user\", \"group\") VALUES ({p1}, {p2})"),
            &[
                djangors_db::BindValue::I64(staff_user.id),
                djangors_db::BindValue::I64(group_id),
            ],
        )
        .await
        .unwrap();

    let req4 = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let req_res4 = router.handle(req4).await.unwrap();
    assert_eq!(req_res4.status(), StatusCode::OK);

    // Test 5: admin_index with a non-superuser staff user who has view permission for exactly one of two registered test models -> body contains link for ModelA but not ModelB
    let req5 = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res5 = router.handle(req5).await.unwrap();
    assert_eq!(res5.status(), StatusCode::OK);
    let body5 = String::from_utf8(res5.body().to_vec()).unwrap();
    assert!(body5.contains("admin_test/modela/"));
    assert!(!body5.contains("admin_test/modelb/"));

    // Clean up group membership/permissions to show that superuser succeeds without any records
    db.conn()
        .execute("DELETE FROM auth_user_groups", &[])
        .await
        .unwrap();
    db.conn()
        .execute("DELETE FROM auth_group_permissions", &[])
        .await
        .unwrap();
    db.conn()
        .execute("DELETE FROM auth_group", &[])
        .await
        .unwrap();
    db.conn()
        .execute("DELETE FROM auth_permission", &[])
        .await
        .unwrap();

    // Test 6: A superuser -> every action succeeds without needing any auth_permission/auth_user_permissions rows at all
    let superuser = User {
        id: 0,
        username: "superuser".to_string(),
        email: "superuser@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let super_session = djangors_sessions::Session::new_empty();
    super_session.set(SESSION_USER_ID_KEY, superuser.id);
    let mut super_ext = Extensions::new();
    super_ext.insert(super_session);

    // GET changelist succeeds
    let req6_get = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(super_ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res6_get = router.handle(req6_get).await.unwrap();
    assert_eq!(res6_get.status(), StatusCode::OK);

    // POST add succeeds
    let mut super_headers = HeaderMap::new();
    super_headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req6_post = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/add/"),
        super_headers,
        Bytes::from("name=SuperuserAdd"),
    )
    .with_extensions(super_ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res6_post = router.handle(req6_post).await.unwrap();
    assert_eq!(res6_post.status(), StatusCode::FOUND);

    // Clean up
    for table in drop_tables {
        let _ = db
            .conn()
            .execute(&format!("DROP TABLE IF EXISTS {}", table), &[])
            .await;
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_csrf_token_wiring() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Create auth_user and test tables if they don't exist
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_model_c_sql = format!(
        "CREATE TABLE test_model_c (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_c_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_csrf".to_string(),
        email: "staff_csrf@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    // Save a test row
    let _ = djangors_orm::queryset::QuerySet::<ModelA>::insert_raw(
        db,
        vec![("name", djangors_orm::expr::Value::Text("test_row".into()))],
    )
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register::<ModelA>();

    let router = Router::new().mount("/admin", site.urls());

    let test_token = "my-awesome-test-csrf-token-12345";
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);

    let make_ext = || {
        let mut ext = Extensions::new();
        ext.insert(session.clone());
        ext.insert(djangors_core::middleware::CsrfToken(test_token.to_string()));
        ext
    };

    // 1. Changelist
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(make_ext())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("name=\"csrfmiddlewaretoken\""));
    assert!(body.contains(&format!("value=\"{}\"", test_token)));

    // 2. Add
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/add/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(make_ext())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("name=\"csrfmiddlewaretoken\""));
    assert!(body.contains(&format!("value=\"{}\"", test_token)));

    // 3. Change
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/1/change/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(make_ext())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("name=\"csrfmiddlewaretoken\""));
    assert!(body.contains(&format!("value=\"{}\"", test_token)));

    // 4. Delete Confirm
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/1/delete/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(make_ext())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("name=\"csrfmiddlewaretoken\""));
    assert!(body.contains(&format!("value=\"{}\"", test_token)));

    // 5. Bulk Delete Confirm (POST to bulk-delete/ with selected pk)
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers,
        Bytes::from("selected=1"),
    )
    .with_extensions(make_ext())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("name=\"csrfmiddlewaretoken\""));
    assert!(body.contains(&format!("value=\"{}\"", test_token)));

    // Cleanup
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
async fn test_favicon_serving() {
    let router = favicon_routes(Router::new());

    let test_cases = vec![
        ("/favicon.ico", "image/x-icon"),
        ("/favicon-16x16.png", "image/png"),
        ("/favicon-32x32.png", "image/png"),
        ("/apple-touch-icon.png", "image/png"),
        ("/android-chrome-192x192.png", "image/png"),
        ("/android-chrome-512x512.png", "image/png"),
        ("/manifest.json", "application/json; charset=utf-8"),
    ];

    for (path, expected_ct) in test_cases {
        let req = Request::new(
            Method::GET,
            Uri::try_from(path).unwrap(),
            HeaderMap::new(),
            Bytes::new(),
        );
        let res = router.handle(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "Failed for path: {}", path);
        assert_eq!(
            res.headers().get("content-type").unwrap().to_str().unwrap(),
            expected_ct
        );
        assert!(!res.body().is_empty());
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_branding_overrides() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // AdminSite with logo and accent color set
    let site = AdminSite::new()
        .with_logo_url("/static/my-logo.png")
        .with_accent_color("#ff0000");

    // AdminSite with defaults (no builders)
    let default_site = AdminSite::new();

    let router = Router::new()
        .mount("/admin-brand", site.urls())
        .mount("/admin-default", default_site.urls());

    // Create a staff user for authenticating
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_brand".to_string(),
        email: "staff_brand@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    // Assert branding is rendered for the customized site
    let req_brand = Request::new(
        Method::GET,
        Uri::from_static("/admin-brand/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_brand = router.handle(req_brand).await.unwrap();
    assert_eq!(res_brand.status(), StatusCode::OK);
    let body_brand = String::from_utf8(res_brand.body().to_vec()).unwrap();
    assert!(body_brand.contains("class=\"site-logo\""));
    assert!(body_brand.contains("src=\"/static/my-logo.png\""));
    assert!(body_brand.contains("<style>:root { --accent: #ff0000; }</style>"));

    // Assert branding is NOT rendered for the default site
    let req_default = Request::new(
        Method::GET,
        Uri::from_static("/admin-default/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_default = router.handle(req_default).await.unwrap();
    assert_eq!(res_default.status(), StatusCode::OK);
    let body_default = String::from_utf8(res_default.body().to_vec()).unwrap();
    assert!(!body_default.contains("class=\"site-logo\""));
    assert!(!body_default.contains("src=\"/static/my-logo.png\""));
    assert!(!body_default.contains("<style>:root { --accent:"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_audit_log_add_change_delete() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_audit".to_string(),
        email: "staff_audit@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());

    let site = AdminSite::new();
    site.register::<ModelA>();
    let router = Router::new().mount("/admin", site.urls());

    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req_add = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/add/"),
        headers.clone(),
        Bytes::from("name=Row1"),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_add = router.handle(req_add).await.unwrap();
    assert_eq!(res_add.status(), StatusCode::FOUND);

    let row = djangors_orm::queryset::QuerySet::<ModelA>::new()
        .filter(djangors_orm::q!(name = "Row1"))
        .unwrap()
        .get(db)
        .await
        .unwrap();
    let pk = row.id;

    let req_change = Request::new(
        Method::POST,
        Uri::try_from(format!("/admin/admin_test/modela/{}/change/", pk)).unwrap(),
        headers.clone(),
        Bytes::from("name=Row1Changed"),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_change = router.handle(req_change).await.unwrap();
    assert_eq!(res_change.status(), StatusCode::FOUND);

    let req_delete = Request::new(
        Method::POST,
        Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", pk)).unwrap(),
        headers,
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_delete = router.handle(req_delete).await.unwrap();
    assert_eq!(res_delete.status(), StatusCode::FOUND);

    let logs = djangors_orm::queryset::QuerySet::<LogEntry>::new()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();

    assert_eq!(logs.len(), 3);
    assert_eq!(logs[0].action_flag, ACTION_ADDITION);
    assert_eq!(logs[0].object_repr, format!("ModelA object ({})", pk));
    assert_eq!(logs[1].action_flag, ACTION_CHANGE);
    assert_eq!(logs[1].object_repr, format!("ModelA object ({})", pk));
    assert_eq!(logs[2].action_flag, ACTION_DELETION);
    assert_eq!(logs[2].object_repr, format!("ModelA object ({})", pk));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_audit_log_bulk_delete_and_list_editable() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_bulk".to_string(),
        email: "staff_bulk@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());

    let pk1 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("R1".to_string())),
            ("content", djangors_orm::expr::Value::Text("C1".to_string())),
        ],
    )
    .await
    .unwrap();

    let pk2 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("R2".to_string())),
            ("content", djangors_orm::expr::Value::Text("C2".to_string())),
        ],
    )
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        list_display: Some(&["name", "content"]),
        list_editable: Some(&["content"]),
        ..Default::default()
    });
    let router = Router::new().mount("/admin", site.urls());

    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );

    let req_bulk = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/bulk-delete/"),
        headers.clone(),
        Bytes::from(format!("selected={}&selected={}&confirm=1", pk1, pk2)),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_bulk = router.handle(req_bulk).await.unwrap();
    assert_eq!(res_bulk.status(), StatusCode::FOUND);

    let logs = djangors_orm::queryset::QuerySet::<LogEntry>::new()
        .filter(djangors_orm::q!(action_flag = 3))
        .unwrap()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(logs.len(), 2);

    let _ = db
        .conn()
        .execute("DELETE FROM djangors_admin_log", &[])
        .await;
    let _ = db.conn().execute("DELETE FROM test_model_d", &[]).await;

    let pk3 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("R3".to_string())),
            ("content", djangors_orm::expr::Value::Text("C3".to_string())),
        ],
    )
    .await
    .unwrap();

    let pk4 = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            ("name", djangors_orm::expr::Value::Text("R4".to_string())),
            ("content", djangors_orm::expr::Value::Text("C4".to_string())),
        ],
    )
    .await
    .unwrap();

    let req_edit = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modeld/save-changelist/"),
        headers,
        Bytes::from(format!(
            "edit-{}-content=UpdatedValid&edit-{}-content=",
            pk3, pk4
        )),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_edit = router.handle(req_edit).await.unwrap();
    assert_eq!(res_edit.status(), StatusCode::OK);

    let logs = djangors_orm::queryset::QuerySet::<LogEntry>::new()
        .filter(djangors_orm::q!(action_flag = 2))
        .unwrap()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].object_id, pk3);

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_admin_index_recent_actions() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let user1 = User {
        id: 0,
        username: "staff_act1".to_string(),
        email: "staff_act1@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let user2 = User {
        id: 0,
        username: "staff_act2".to_string(),
        email: "staff_act2@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let _log1 = LogEntry {
        id: 0,
        user_id: user1.id,
        action_time: now - chrono::Duration::try_seconds(10).unwrap(),
        app_label: "admin_test".to_string(),
        model_name: "ModelA".to_string(),
        object_id: 100,
        object_repr: "ModelA object (100)".to_string(),
        action_flag: ACTION_ADDITION,
        change_message: "Added.".to_string(),
        field_diff: None,
    }
    .save(db)
    .await
    .unwrap();

    let _log2 = LogEntry {
        id: 0,
        user_id: user1.id,
        action_time: now - chrono::Duration::try_seconds(5).unwrap(),
        app_label: "admin_test".to_string(),
        model_name: "ModelA".to_string(),
        object_id: 101,
        object_repr: "ModelA object (101)".to_string(),
        action_flag: ACTION_CHANGE,
        change_message: "Changed.".to_string(),
        field_diff: None,
    }
    .save(db)
    .await
    .unwrap();

    let site = AdminSite::new();
    let router = Router::new().mount("/admin", site.urls());

    let session_user1 = djangors_sessions::Session::new_empty();
    session_user1.set(SESSION_USER_ID_KEY, user1.id);
    let mut ext1 = Extensions::new();
    ext1.insert(session_user1);

    let req_user1 = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext1)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res_user1 = router.handle(req_user1).await.unwrap();
    assert_eq!(res_user1.status(), StatusCode::OK);
    let body1 = String::from_utf8(res_user1.body().to_vec()).unwrap();

    assert!(body1.contains("Recent actions"));
    assert!(body1.contains("ModelA object (100)"));
    assert!(body1.contains("ModelA object (101)"));

    let idx101 = body1.find("ModelA object (101)").unwrap();
    let idx100 = body1.find("ModelA object (100)").unwrap();
    assert!(idx101 < idx100);

    let session_user2 = djangors_sessions::Session::new_empty();
    session_user2.set(SESSION_USER_ID_KEY, user2.id);
    let mut ext2 = Extensions::new();
    ext2.insert(session_user2);

    let req_user2 = Request::new(
        Method::GET,
        Uri::from_static("/admin/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext2)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));

    let res_user2 = router.handle(req_user2).await.unwrap();
    assert_eq!(res_user2.status(), StatusCode::OK);
    let body2 = String::from_utf8(res_user2.body().to_vec()).unwrap();

    assert!(!body2.contains("Recent actions"));
    assert!(!body2.contains("ModelA object (100)"));
    assert!(!body2.contains("ModelA object (101)"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_fieldsets() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_fieldsets".to_string(),
        email: "staff_fieldsets@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        fieldsets: Some(&[("Section A", &["name"]), ("Section B", &["content"])]),
        ..Default::default()
    });
    let router = Router::new().mount("/admin", site.urls());

    let req_add = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modeld/add/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req_add).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("Section A"));
    assert!(body.contains("Section B"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_readonly_fields() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_ro".to_string(),
        email: "staff_ro@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register_with::<ModelD>(ModelAdminConfig {
        readonly_fields: Some(&["name"]),
        ..Default::default()
    });
    let router = Router::new().mount("/admin", site.urls());

    let row = ModelD {
        id: 0,
        name: "ro_test".to_string(),
        content: "ro_content".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let req_change = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modeld/{}/change/", row.id)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req_change).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("(readonly):"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_raw_id_fields() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_model_c_sql = format!(
        "CREATE TABLE test_model_c (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_c_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_rawid".to_string(),
        email: "staff_rawid@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register_with::<ModelC>(ModelAdminConfig {
        raw_id_fields: Some(&["parent"]),
        ..Default::default()
    });
    let router = Router::new().mount("/admin", site.urls());

    let a = ModelA {
        id: 0,
        name: "raw_target".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _c = ModelC {
        id: 0,
        parent: djangors_orm::ForeignKey::new(a.id),
    }
    .save(db)
    .await
    .unwrap();

    let req_changelist = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modelc/add/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req_changelist).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("Look up"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_transitive_walk_delete() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_g", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_model_c_sql = format!(
        "CREATE TABLE test_model_c (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_c_sql, &[]).await.unwrap();

    let create_model_g_sql = format!(
        "CREATE TABLE test_model_g (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_g_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_tw".to_string(),
        email: "staff_tw@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelC>();
    site.register::<ModelG>();
    let router = Router::new().mount("/admin", site.urls());

    let a = ModelA {
        id: 0,
        name: "transitive_target".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let c = ModelC {
        id: 0,
        parent: djangors_orm::ForeignKey::new(a.id),
    }
    .save(db)
    .await
    .unwrap();
    let _g = ModelG {
        id: 0,
        parent: djangors_orm::ForeignKey::new(c.id),
    }
    .save(db)
    .await
    .unwrap();

    let req_delete_get = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", a.id)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req_delete_get).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("ModelC"));
    assert!(body.contains("ModelG"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_c", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_g", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_protect_single_delete_400() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_p", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_model_p_sql = format!(
        "CREATE TABLE test_model_p (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_p_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_prot".to_string(),
        email: "staff_prot@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelP>();
    let router = Router::new().mount("/admin", site.urls());

    let a = ModelA {
        id: 0,
        name: "protect_target".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _p = ModelP {
        id: 0,
        parent: djangors_orm::ForeignKey::new(a.id),
    }
    .save(db)
    .await
    .unwrap();

    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req_delete_post = Request::new(
        Method::POST,
        Uri::try_from(format!("/admin/admin_test/modela/{}/delete/", a.id)).unwrap(),
        headers,
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req_delete_post).await;
    match res {
        Ok(r) => assert_eq!(r.status(), StatusCode::BAD_REQUEST),
        Err(e) => assert!(e.to_string().contains("protected")),
    }

    let p1 = db.dialect().placeholder(1);
    let still_exists: i64 = db
        .conn()
        .fetch_one(
            &format!("SELECT COUNT(*) FROM test_model_a WHERE id = {p1}"),
            &[djangors_db::BindValue::I64(a.id)],
        )
        .await
        .unwrap()
        .try_i64(0)
        .unwrap()
        .unwrap();
    assert_eq!(still_exists, 1);

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_p", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_protect_bulk_delete_skip() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_p", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_model_p_sql = format!(
        "CREATE TABLE test_model_p (
                id {auto_pk},
                parent BIGINT NOT NULL
            )"
    );
    db.conn().execute(&create_model_p_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_bulkp".to_string(),
        email: "staff_bulkp@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register::<ModelA>();
    site.register::<ModelP>();
    let router = Router::new().mount("/admin", site.urls());

    let a1 = ModelA {
        id: 0,
        name: "bulk_protected".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let a2 = ModelA {
        id: 0,
        name: "bulk_unprotected".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let _p = ModelP {
        id: 0,
        parent: djangors_orm::ForeignKey::new(a1.id),
    }
    .save(db)
    .await
    .unwrap();

    let body = format!("confirm=1&selected={}&selected={}", a1.id, a2.id);
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req_bulk_delete = Request::new(
        Method::POST,
        Uri::from_static("/admin/admin_test/modela/bulk-delete/"),
        headers,
        Bytes::from(body),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req_bulk_delete).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);

    let remaining: i64 = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM test_model_a", &[])
        .await
        .unwrap()
        .try_i64(0)
        .unwrap()
        .unwrap();
    assert_eq!(remaining, 1);

    let still_alive: i64 = db
        .conn()
        .fetch_one(
            "SELECT COUNT(*) FROM test_model_a WHERE name = 'bulk_protected'",
            &[],
        )
        .await
        .unwrap()
        .try_i64(0)
        .unwrap()
        .unwrap();
    assert_eq!(still_alive, 1);

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_p", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_base_filter() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_bf".to_string(),
        email: "staff_bf@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let _a1 = ModelA {
        id: 0,
        name: "apple".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let _a2 = ModelA {
        id: 0,
        name: "banana".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register_with::<ModelA>(ModelAdminConfig {
        search_fields: Some(&["name"]),
        base_filter: Some(djangors_orm::UnresolvedExpr::And(vec![
            djangors_orm::UnresolvedCompare {
                field: "name",
                value: djangors_orm::Value::Text("apple".to_string()),
            },
        ])),
        ..Default::default()
    });
    let router = Router::new().mount("/admin", site.urls());

    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/admin_test/modela/?q=apple"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("apple"));
    assert!(!body.contains("banana"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_extra_route() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_er".to_string(),
        email: "staff_er@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let extra = Router::new().get(
        "/my-custom/",
        |_req: Request, _params: PathParams| async move {
            Ok(Response::text(StatusCode::OK, "custom route works"))
        },
    );

    let site = AdminSite::new().extra_route(extra);
    site.register::<ModelA>();
    let router = Router::new().mount("/admin", site.urls());

    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/my-custom/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("custom route works"));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_history_page() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_a_sql = format!(
        "CREATE TABLE test_model_a (
                id {auto_pk},
                name VARCHAR(255) NOT NULL
            )"
    );
    db.conn().execute(&create_model_a_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_hist".to_string(),
        email: "staff_hist@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let a = ModelA {
        id: 0,
        name: "hist_test".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let _log = LogEntry {
        id: 0,
        user_id: staff.id,
        action_time: now,
        app_label: "admin_test".to_string(),
        model_name: "ModelA".to_string(),
        object_id: a.id,
        object_repr: format!("ModelA object ({})", a.id),
        action_flag: ACTION_ADDITION,
        change_message: "Created.".to_string(),
        field_diff: None,
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session);

    let site = AdminSite::new();
    site.register::<ModelA>();
    let router = Router::new().mount("/admin", site.urls());

    let req = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modela/{}/history/", a.id)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();

    assert!(body.contains("History for ModelA"));
    assert!(body.contains("staff_hist"));
    assert!(body.contains("Created."));

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_a", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_phase5_groups_permissions_admin_ui() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let drop_tables = ["auth_user_groups", "auth_group", "auth_user"];
    for table in drop_tables {
        let _ = db
            .conn()
            .execute(&format!("DROP TABLE IF EXISTS {}", table), &[])
            .await;
    }

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_auth_group_sql = format!(
        "CREATE TABLE auth_group (
                id {auto_pk},
                name VARCHAR(150) NOT NULL UNIQUE
            )"
    );
    db.conn()
        .execute(&create_auth_group_sql, &[])
        .await
        .unwrap();

    let create_auth_user_groups_sql = format!(
        "CREATE TABLE auth_user_groups (
                id {auto_pk},
                \"user\" BIGINT NOT NULL,
                \"group\" BIGINT NOT NULL
            )"
    );
    db.conn()
        .execute(&create_auth_user_groups_sql, &[])
        .await
        .unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_groups".to_string(),
        email: "staff_groups@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, staff.id);

    let site = AdminSite::new();
    site.register::<djangors_auth::Permission>();
    site.register_with::<djangors_auth::Group>(ModelAdminConfig {
        search_fields: Some(&["name"]),
        ..Default::default()
    });
    site.register::<djangors_auth::UserGroup>();
    site.register::<djangors_auth::GroupPermission>();
    site.register::<djangors_auth::UserPermission>();
    let router = Router::new().mount("/admin", site.urls());

    // 1. Create a Group through the generic admin add/ page.
    let mut ext = Extensions::new();
    ext.insert(session.clone());
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/djangors_auth/group/add/"),
        headers,
        Bytes::from("name=Admins"),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);

    let group = djangors_orm::queryset::QuerySet::<djangors_auth::Group>::new()
        .filter(djangors_orm::q!(name = "Admins"))
        .unwrap()
        .get(db)
        .await
        .unwrap();

    // 2. Add a UserGroup row (group membership) through the generic admin.
    let mut ext = Extensions::new();
    ext.insert(session.clone());
    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let req = Request::new(
        Method::POST,
        Uri::from_static("/admin/djangors_auth/usergroup/add/"),
        headers,
        Bytes::from(format!("user={}&group={}", staff.id, group.id)),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FOUND);

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let membership_count: i64 = db
        .conn()
        .fetch_one(
            &format!(
                "SELECT COUNT(*) FROM auth_user_groups WHERE \"user\" = {p1} AND \"group\" = {p2}"
            ),
            &[
                djangors_db::BindValue::I64(staff.id),
                djangors_db::BindValue::I64(group.id),
            ],
        )
        .await
        .unwrap()
        .try_i64(0)
        .unwrap()
        .unwrap();
    assert_eq!(membership_count, 1);

    // 3. Changelist pages for both models render through the generic admin.
    let mut ext = Extensions::new();
    ext.insert(session.clone());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/admin/djangors_auth/group/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = String::from_utf8(res.body().to_vec()).unwrap();
    assert!(body.contains("Admins"));

    let drop_tables = ["auth_user_groups", "auth_group", "auth_user"];
    for table in drop_tables {
        let _ = db
            .conn()
            .execute(&format!("DROP TABLE IF EXISTS {}", table), &[])
            .await;
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_audit_diffing_and_history_rendering() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_model_d", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS djangors_admin_log", &[])
        .await;

    let create_auth_user_sql = format!(
        "CREATE TABLE auth_user (
                id {auto_pk},
                username VARCHAR(150) NOT NULL,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts_type} NOT NULL,
                last_login {ts_type}
            )"
    );
    db.conn().execute(&create_auth_user_sql, &[]).await.unwrap();

    let create_model_d_sql = format!(
        "CREATE TABLE test_model_d (
                id {auto_pk},
                name TEXT NOT NULL,
                content TEXT NOT NULL
            )"
    );
    db.conn().execute(&create_model_d_sql, &[]).await.unwrap();

    let create_admin_log_sql = format!(
        "CREATE TABLE djangors_admin_log (
                id {auto_pk},
                user_id BIGINT NOT NULL,
                action_time {ts_type} NOT NULL,
                app_label VARCHAR(100) NOT NULL,
                model_name VARCHAR(100) NOT NULL,
                object_id BIGINT NOT NULL,
                object_repr VARCHAR(200) NOT NULL,
                action_flag INTEGER NOT NULL,
                change_message TEXT NOT NULL,
                field_diff TEXT
            )"
    );
    db.conn().execute(&create_admin_log_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let staff = User {
        id: 0,
        username: "staff_diff".to_string(),
        email: "staff_diff@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: now,
        last_login: Some(now),
    }
    .save(db)
    .await
    .unwrap();

    let session_staff = djangors_sessions::Session::new_empty();
    session_staff.set(SESSION_USER_ID_KEY, staff.id);
    let mut ext = Extensions::new();
    ext.insert(session_staff.clone());

    let pk = djangors_orm::queryset::QuerySet::<ModelD>::insert_raw(
        db,
        vec![
            (
                "name",
                djangors_orm::expr::Value::Text("OrigName".to_string()),
            ),
            (
                "content",
                djangors_orm::expr::Value::Text("OrigContent".to_string()),
            ),
        ],
    )
    .await
    .unwrap();

    let site = AdminSite::new();
    site.register::<ModelD>();
    let router = Router::new().mount("/admin", site.urls());

    let mut headers = HeaderMap::new();
    headers.insert(
        hyper::http::header::CONTENT_TYPE,
        "application/x-www-form-urlencoded".parse().unwrap(),
    );

    // Submit form changing ONLY 'name', leaving 'content' unchanged ("OrigContent")
    let req_change = Request::new(
        Method::POST,
        Uri::try_from(format!("/admin/admin_test/modeld/{}/change/", pk)).unwrap(),
        headers,
        Bytes::from("name=NewName&content=OrigContent"),
    )
    .with_extensions(ext.clone())
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_change = router.handle(req_change).await.unwrap();
    assert_eq!(res_change.status(), StatusCode::FOUND);

    let logs = djangors_orm::queryset::QuerySet::<LogEntry>::new()
        .filter(djangors_orm::q!(action_flag = 2))
        .unwrap()
        .order_by("id")
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);

    let raw_diff = logs[0]
        .field_diff
        .as_ref()
        .expect("field_diff should be present");
    let diff_items: Vec<FieldDiffItem> = serde_json::from_str(raw_diff).unwrap();
    assert_eq!(diff_items.len(), 1);
    assert_eq!(diff_items[0].field, "name");
    assert_eq!(diff_items[0].old, "OrigName");
    assert_eq!(diff_items[0].new, "NewName");

    // Verify history page renders the diff block
    let req_hist = Request::new(
        Method::GET,
        Uri::try_from(format!("/admin/admin_test/modeld/{}/history/", pk)).unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(ext)
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res_hist = router.handle(req_hist).await.unwrap();
    assert_eq!(res_hist.status(), StatusCode::OK);
    let body_hist = String::from_utf8(res_hist.body().to_vec()).unwrap();
    assert!(body_hist.contains("name: OrigName"));
    assert!(body_hist.contains("NewName"));

    let drop_tables = ["test_model_d", "auth_user", "djangors_admin_log"];
    for table in drop_tables {
        let _ = db
            .conn()
            .execute(&format!("DROP TABLE IF EXISTS {}", table), &[])
            .await;
    }
}

#[tokio::test]
async fn test_date_hierarchy_drilldown_sqlite() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:");
    let db = djangors_db::Database::connect(&config).await.unwrap();
    db.conn()
        .execute(
            "CREATE TABLE test_events (id INTEGER PRIMARY KEY, created_at TEXT NOT NULL)",
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_events (id, created_at) VALUES (1, '2023-05-15 10:00:00')",
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            "INSERT INTO test_events (id, created_at) VALUES (2, '2025-11-20 14:30:00')",
            &[],
        )
        .await
        .unwrap();

    let years = date_hierarchy_drilldown_values(&db, "test_events", "created_at", None, None)
        .await
        .unwrap();

    assert_eq!(years, vec![2023, 2025]);

    let months =
        date_hierarchy_drilldown_values(&db, "test_events", "created_at", Some(2023), None)
            .await
            .unwrap();

    assert_eq!(months, vec![5]);

    let days =
        date_hierarchy_drilldown_values(&db, "test_events", "created_at", Some(2023), Some(5))
            .await
            .unwrap();

    assert_eq!(days, vec![15]);
}
