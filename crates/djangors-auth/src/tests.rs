use super::*;
use djangors_orm::q;
use djangors_orm::Model;

static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

static CROSS_PROCESS_LOCK: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

/// See the identical helper in `djangors-admin`'s test module for the full
/// rationale (task #61) - both crates' tests race to DROP/CREATE the same
/// `auth_user` table, and the same cross-process lock name is used here
/// deliberately so the two crates' test binaries actually exclude each other,
/// not just their own internal tests.
async fn ensure_cross_process_lock_held() {
    if djangors_test::TestBackend::current() == djangors_test::TestBackend::Sqlite {
        return;
    }
    CROSS_PROCESS_LOCK
        .get_or_init(|| async {
            let db = djangors_db::Database::connect(&djangors_db::config::DatabaseConfig::new(
                "postgres://postgres:postgres@localhost/djangors_test",
            ))
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

#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "test_auth_user")]
pub struct TestUser {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 150)]
    pub username: String,

    #[djangors(max_length = 254)]
    pub email: String,

    pub password: String,

    pub is_active: bool,
    pub is_staff: bool,
    pub is_superuser: bool,

    pub date_joined: chrono::DateTime<chrono::Utc>,
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait::async_trait]
impl AuthUser for TestUser {
    fn id(&self) -> i64 {
        self.id
    }

    fn username(&self) -> &str {
        &self.username
    }

    fn password_hash(&self) -> &str {
        &self.password
    }

    fn set_password_hash(&mut self, hash: String) {
        self.password = hash;
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn is_superuser(&self) -> bool {
        self.is_superuser
    }

    async fn update_user(&self, db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
        self.update(db).await
    }
}

#[test]
fn test_user_derives_and_metadata() {
    let meta = User::meta();
    assert_eq!(meta.struct_name, "User");
    assert_eq!(meta.app_label, "djangors_auth");
    assert_eq!(meta.table_name, "auth_user");

    // Check that we have the expected number of fields
    assert_eq!(meta.fields.len(), 9);
}

#[test]
fn test_hash_password_properties() {
    let password = "my_secure_password";
    let hash1 = hash_password(password).expect("hashing should succeed");

    // Must start with $argon2id$
    assert!(
        hash1.starts_with("$argon2id$"),
        "Hash '{}' should start with $argon2id$",
        hash1
    );

    // Calling twice must produce different hashes (random salt)
    let hash2 = hash_password(password).expect("hashing should succeed");
    assert_ne!(
        hash1, hash2,
        "Hashes should be different due to random salt"
    );
}

#[test]
fn test_verify_password_correct_and_incorrect() {
    let password = "my_secure_password";
    let hash = hash_password(password).expect("hashing should succeed");

    // Correct password
    let result_correct = verify_password(password, &hash).expect("verification should run");
    assert!(result_correct);

    // Incorrect password
    let result_incorrect =
        verify_password("wrong_password", &hash).expect("verification should run");
    assert!(!result_incorrect);
}

#[test]
fn test_verify_password_malformed_hash() {
    // Malformed/corrupted hash string should return Err
    let malformed_hashes = vec![
        "not_a_hash",
        "$argon2id$v=19$m=4096,t=3,p=1$invalid_salt$invalid_hash",
        "",
    ];

    for hash in malformed_hashes {
        let result = verify_password("password", hash);
        assert!(
            result.is_err(),
            "Expected error for malformed hash '{}', got {:?}",
            hash,
            result
        );
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_user_db_round_trip() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Clean up test table
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_auth_user", &[])
        .await;

    // Create test table
    let create_sql = format!(
        "CREATE TABLE test_auth_user (
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
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // Construct a User with a hashed password
    let plaintext = "supersecret";
    let hash = hash_password(plaintext).unwrap();
    let now = chrono::Utc::now();

    let mut user = TestUser {
        id: 0,
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: "".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    user.set_password_hash(hash);

    // Save user
    let saved_user = user.save(db).await.unwrap();
    assert_ne!(saved_user.id, 0);
    assert_eq!(saved_user.username, "testuser");
    assert_eq!(saved_user.email, "test@example.com");
    assert!(saved_user.is_active);
    assert_eq!(saved_user.date_joined.timestamp(), now.timestamp());
    assert_eq!(saved_user.last_login.unwrap().timestamp(), now.timestamp());

    // Fetch user back
    let fetched_user = TestUser::objects()
        .filter(q!(id = saved_user.id))
        .unwrap()
        .get(db)
        .await
        .unwrap();

    // Verify password against round-tripped password field
    let verified = verify_password(plaintext, fetched_user.password_hash()).unwrap();
    assert!(verified);

    // Cleanup
    let _ = db.conn().execute("DROP TABLE test_auth_user", &[]).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_model_backend_authenticate() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_sql = format!(
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
    db.conn().execute(&create_sql, &[]).await.unwrap();

    let plaintext = "correct_password";
    let hash = hash_password(plaintext).unwrap();
    let now = chrono::Utc::now();

    // 1. Create an active user
    let active_user_raw = User {
        id: 0,
        username: "active_user".to_string(),
        email: "active@example.com".to_string(),
        password: hash.clone(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let _active_user = active_user_raw.save(db).await.unwrap();

    // 2. Create an inactive user
    let inactive_user_raw = User {
        id: 0,
        username: "inactive_user".to_string(),
        email: "inactive@example.com".to_string(),
        password: hash.clone(),
        is_active: false,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let _ = inactive_user_raw.save(db).await.unwrap();

    let backend = ModelBackend;

    // Test correct credentials -> Some(user)
    let auth_res = backend
        .authenticate(db, "active_user", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_some());
    assert_eq!(auth_res.unwrap().username, "active_user");

    // Test wrong password -> None
    let auth_res = backend
        .authenticate(db, "active_user", "wrong_password")
        .await
        .unwrap();
    assert!(auth_res.is_none());

    // Test nonexistent username -> None
    let auth_res = backend
        .authenticate(db, "nonexistent", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_none());

    // Test inactive user with correct password -> None
    let auth_res = backend
        .authenticate(db, "inactive_user", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_none());

    // Cleanup
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
}

#[test]
fn test_login_session_mechanics() {
    let secret = b"super-secret-key-for-testing-purposes-only";
    let store = djangors_sessions::SignedCookieStore::new(secret);
    let session = djangors_sessions::Session::new_empty();

    let user = User {
        id: 42,
        username: "testuser".to_string(),
        email: "test@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: chrono::Utc::now(),
        last_login: None,
    };

    let cookie_val_before = store.encode(&session);

    login(&session, &user);

    let cookie_val_after = store.encode(&session);

    // Prove pre-rotation and post-rotation cookie strings are different (cycle_key logic)
    assert_ne!(cookie_val_before, cookie_val_after);

    // Verify session sets the _auth_user_id
    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(42));
}

#[tokio::test]
async fn test_logout_session_mechanics() {
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, 42i64);
    session.set("other_key", "value".to_string());

    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), Some(42));

    logout(&session).await;

    assert_eq!(session.get::<i64>(SESSION_USER_ID_KEY), None);
    assert!(session.is_empty());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_auth_extractor() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    use bytes::Bytes;
    use djangors_core::extract::FromRequest;
    use djangors_core::Request;
    use hyper::http::{Extensions, HeaderMap, Method, Uri};

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_sql = format!(
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
    db.conn().execute(&create_sql, &[]).await.unwrap();

    let now = chrono::Utc::now();
    let active_user_raw = User {
        id: 0,
        username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        password: "hash".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let active_user = active_user_raw.save(db).await.unwrap();

    let inactive_user_raw = User {
        id: 0,
        username: "inactive".to_string(),
        email: "inactive@example.com".to_string(),
        password: "hash".to_string(),
        is_active: false,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let inactive_user = inactive_user_raw.save(db).await.unwrap();

    // 1. no session extension present -> Err(Unauthorized)
    let req_no_session = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::default());
    let res = Auth::<User>::from_request(&req_no_session).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // 2. session present but no _auth_user_id set -> Err(Unauthorized)
    let session = djangors_sessions::Session::new_empty();
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_empty_session = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(djangors_core::state::AppState::default());
    let res = Auth::<User>::from_request(&req_empty_session).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // 3. valid _auth_user_id pointing at a real active user -> Ok(Auth(user))
    let app_state = djangors_core::state::AppState::new().insert(db.clone());

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, active_user.id);
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_valid = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(app_state.clone());
    let res = Auth::<User>::from_request(&req_valid).await;
    assert!(res.is_ok());
    assert_eq!(res.unwrap().0.username, "alice");

    // 4. valid _auth_user_id pointing at an inactive user -> Err(Unauthorized)
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, inactive_user.id);
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_inactive = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(app_state.clone());
    let res = Auth::<User>::from_request(&req_inactive).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // 5. valid _auth_user_id pointing at a since-deleted row -> Err(Unauthorized)
    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, 9999i64); // Nonexistent ID
    let mut extensions = Extensions::new();
    extensions.insert(session.clone());
    let req_deleted = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_extensions(extensions)
    .with_state(app_state.clone());
    let res = Auth::<User>::from_request(&req_deleted).await;
    assert!(res.is_err());
    assert!(matches!(
        res.err().unwrap(),
        djangors_core::error::DjangorsError::Unauthorized(_)
    ));

    // Cleanup
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_audit_signals_succeeded_and_failed() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;

    let create_sql = format!(
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
    db.conn().execute(&create_sql, &[]).await.unwrap();

    let plaintext = "correct_password";
    let hash = hash_password(plaintext).unwrap();
    let now = chrono::Utc::now();

    // 1. Create an active user
    let active_user_raw = User {
        id: 0,
        username: "sig_active_user".to_string(),
        email: "active@example.com".to_string(),
        password: hash.clone(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let _active_user = active_user_raw.save(db).await.unwrap();

    // 2. Create an inactive user
    let inactive_user_raw = User {
        id: 0,
        username: "sig_inactive_user".to_string(),
        email: "inactive@example.com".to_string(),
        password: hash.clone(),
        is_active: false,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: Some(now),
    };
    let _ = inactive_user_raw.save(db).await.unwrap();

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let succeeded_counter = Arc::new(AtomicUsize::new(0));
    let failed_counter = Arc::new(AtomicUsize::new(0));

    let succeeded_clone = succeeded_counter.clone();
    LOGIN_SUCCEEDED.connect(move |payload| {
        let succeeded = succeeded_clone.clone();
        async move {
            if payload.username == "sig_active_user" {
                succeeded.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    let failed_clone = failed_counter.clone();
    LOGIN_FAILED.connect(move |payload| {
        let failed = failed_clone.clone();
        async move {
            if payload.username == "sig_active_user"
                || payload.username == "sig_inactive_user"
                || payload.username == "sig_nonexistent"
            {
                failed.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    let backend = ModelBackend;

    // Test correct credentials -> LOGIN_SUCCEEDED fires
    let auth_res = backend
        .authenticate(db, "sig_active_user", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_some());
    assert_eq!(succeeded_counter.load(Ordering::SeqCst), 1);
    assert_eq!(failed_counter.load(Ordering::SeqCst), 0);

    // Test wrong password -> LOGIN_FAILED fires
    let auth_res = backend
        .authenticate(db, "sig_active_user", "wrong_password")
        .await
        .unwrap();
    assert!(auth_res.is_none());
    assert_eq!(succeeded_counter.load(Ordering::SeqCst), 1);
    assert_eq!(failed_counter.load(Ordering::SeqCst), 1);

    // Test nonexistent username -> LOGIN_FAILED fires
    let auth_res = backend
        .authenticate(db, "sig_nonexistent", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_none());
    assert_eq!(succeeded_counter.load(Ordering::SeqCst), 1);
    assert_eq!(failed_counter.load(Ordering::SeqCst), 2);

    // Test inactive user with correct password -> LOGIN_FAILED fires
    let auth_res = backend
        .authenticate(db, "sig_inactive_user", plaintext)
        .await
        .unwrap();
    assert!(auth_res.is_none());
    assert_eq!(succeeded_counter.load(Ordering::SeqCst), 1);
    assert_eq!(failed_counter.load(Ordering::SeqCst), 3);

    // Cleanup
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
}

#[tokio::test]
async fn test_audit_signal_logged_out() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let logged_out_counter = Arc::new(AtomicUsize::new(0));
    let logged_out_clone = logged_out_counter.clone();

    LOGGED_OUT.connect(move |payload| {
        let logged_out = logged_out_clone.clone();
        async move {
            if payload.user_id == Some(99) {
                logged_out.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    let session = djangors_sessions::Session::new_empty();
    session.set(SESSION_USER_ID_KEY, 99i64);

    logout(&session).await;

    assert_eq!(logged_out_counter.load(Ordering::SeqCst), 1);
}

struct TestDoubleBackend {
    call_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl AuthBackend for TestDoubleBackend {
    type User = User;

    async fn authenticate(
        &self,
        _db: &djangors_db::Database,
        username: &str,
        _password: &str,
    ) -> Result<Option<Self::User>, AuthError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        LOGIN_FAILED
            .send(LoginFailed {
                username: username.to_string(),
            })
            .await;
        Ok(None)
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_persistent_lockout_backend_locks_even_correct_credentials_and_resets_on_expiry() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await;
    let create_user_sql = format!(
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
    db.conn().execute(&create_user_sql, &[]).await.unwrap();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS auth_login_lockout", &[])
        .await;
    let create_lockout_sql = format!(
        "CREATE TABLE auth_login_lockout (
            id {auto_pk},
            username VARCHAR(150) NOT NULL UNIQUE,
            failed_attempts INTEGER NOT NULL,
            first_failed_at {ts_type} NOT NULL,
            locked_until {ts_type}
        )"
    );
    db.conn().execute(&create_lockout_sql, &[]).await.unwrap();

    let plaintext = "correct_password";
    let hash = hash_password(plaintext).unwrap();
    let now = chrono::Utc::now();
    let user = User {
        id: 0,
        username: "lockout_user".to_string(),
        email: "lockout@example.com".to_string(),
        password: hash,
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: now,
        last_login: None,
    };
    user.save(db).await.unwrap();

    let backend =
        PersistentLockoutBackend::new(ModelBackend, 3, std::time::Duration::from_secs(3600));

    // 1st and 2nd failures: wrong password, not yet locked.
    for _ in 0..2 {
        let res = backend
            .authenticate(db, "lockout_user", "wrong")
            .await
            .unwrap();
        assert!(res.is_none());
    }

    // 3rd failure crosses max_attempts=3 - now locked.
    let res = backend
        .authenticate(db, "lockout_user", "wrong")
        .await
        .unwrap();
    assert!(res.is_none());

    // Even the CORRECT password is now rejected - this is what distinguishes a
    // lockout from plain rate limiting.
    let err = backend
        .authenticate(db, "lockout_user", plaintext)
        .await
        .unwrap_err();
    match err {
        AuthError::AccountLocked { retry_after_secs } => {
            assert!(retry_after_secs > 0 && retry_after_secs <= 3600);
        }
        other => panic!("expected AccountLocked, got {other:?}"),
    }

    // Simulate the lockout window expiring (avoids a real 1-hour test sleep).
    let p1 = db.dialect().placeholder(1);
    let sql = format!(
        "UPDATE auth_login_lockout SET locked_until = {p1} WHERE username = 'lockout_user'"
    );
    let overdue = chrono::Utc::now() - chrono::Duration::minutes(1);
    db.conn()
        .execute(&sql, &[djangors_db::BindValue::DateTime(overdue)])
        .await
        .unwrap();

    // An expired lockout no longer rejects correct credentials, and a successful
    // login clears the failure streak entirely.
    let res = backend
        .authenticate(db, "lockout_user", plaintext)
        .await
        .unwrap();
    assert!(res.is_some());

    let remaining = LoginLockout::objects()
        .filter(djangors_orm::q!(username = "lockout_user"))
        .unwrap()
        .first(db)
        .await
        .unwrap();
    assert!(
        remaining.is_none(),
        "a successful login must clear the lockout row entirely"
    );

    let _ = db
        .conn()
        .execute("DROP TABLE auth_login_lockout", &[])
        .await;
    let _ = db.conn().execute("DROP TABLE auth_user", &[]).await;
}

#[tokio::test]
async fn test_rate_limited_backend_limits_attempts() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let call_count = Arc::new(AtomicUsize::new(0));
    let inner = TestDoubleBackend {
        call_count: call_count.clone(),
    };
    let backend = RateLimitedBackend::new(inner, 3, Duration::from_millis(50));

    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();

    let failed_counter = Arc::new(AtomicUsize::new(0));
    let failed_clone = failed_counter.clone();
    LOGIN_FAILED.connect(move |payload| {
        let failed = failed_clone.clone();
        async move {
            if payload.username == "throttled_user" {
                failed.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    // 1st attempt: success (reaches inner)
    let res = backend.authenticate(db, "throttled_user", "pass").await;
    assert!(res.unwrap().is_none());
    assert_eq!(call_count.load(Ordering::SeqCst), 1);

    // 2nd attempt: success (reaches inner)
    let res = backend.authenticate(db, "throttled_user", "pass").await;
    assert!(res.unwrap().is_none());
    assert_eq!(call_count.load(Ordering::SeqCst), 2);

    // 3rd attempt: success (reaches inner)
    let res = backend.authenticate(db, "throttled_user", "pass").await;
    assert!(res.unwrap().is_none());
    assert_eq!(call_count.load(Ordering::SeqCst), 3);

    // 4th attempt: rate limited (fails without reaching inner)
    let res = backend.authenticate(db, "throttled_user", "pass").await;
    assert!(matches!(res.err().unwrap(), AuthError::RateLimited));
    assert_eq!(call_count.load(Ordering::SeqCst), 3);

    // Verify LOGIN_FAILED fired for the 4th attempt too (rejections also fire it)
    assert_eq!(failed_counter.load(Ordering::SeqCst), 4);

    // Different user is unaffected
    let res_diff = backend.authenticate(db, "other_user", "pass").await;
    assert!(res_diff.unwrap().is_none());
    assert_eq!(call_count.load(Ordering::SeqCst), 4);

    // After window elapses, we can attempt again
    tokio::time::sleep(Duration::from_millis(60)).await;
    let res = backend.authenticate(db, "throttled_user", "pass").await;
    assert!(res.unwrap().is_none());
    assert_eq!(call_count.load(Ordering::SeqCst), 5);
}

struct TestMailBackend {
    sent_messages: std::sync::Mutex<Vec<djangors_mail::Message>>,
}

impl TestMailBackend {
    fn new() -> Self {
        Self {
            sent_messages: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl djangors_mail::MailBackend for TestMailBackend {
    async fn send(&self, message: &djangors_mail::Message) -> Result<(), djangors_mail::MailError> {
        self.sent_messages.lock().unwrap().push(message.clone());
        Ok(())
    }
}

#[test]
fn test_password_reset_token_roundtrip_and_invalidation() {
    let secret = b"my_super_secret_key";
    let mut user = TestUser {
        id: 42,
        username: "test_user".to_string(),
        email: "test@example.com".to_string(),
        password: "hash_prefix_of_some_kind".to_string(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: chrono::Utc::now(),
        last_login: None,
    };

    // 1. token round-trip: valid token verifies successfully
    let token = generate_password_reset_token(&user, secret, Duration::from_secs(3600));
    assert!(verify_password_reset_token(&user, &token, secret));

    // 2. token expired (past ttl) fails to verify
    let expired_token = generate_password_reset_token(&user, secret, Duration::from_secs(0));
    assert!(!verify_password_reset_token(&user, &expired_token, secret));

    // 3. token generated for one user fails to verify against a different user
    let mut other_user = user.clone();
    other_user.id = 99;
    assert!(!verify_password_reset_token(&other_user, &token, secret));

    // 4. token verifies fine before a password change, then FAILS after the user's password_hash changes
    assert!(verify_password_reset_token(&user, &token, secret));
    user.set_password_hash("different_password_hash".to_string());
    assert!(!verify_password_reset_token(&user, &token, secret));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_db_password_reset_flow() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Clean up test table
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS test_auth_user", &[])
        .await;

    // Create test table
    let create_sql = format!(
        "CREATE TABLE test_auth_user (
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
    db.conn().execute(&create_sql, &[]).await.unwrap();

    // Create an active user
    let raw_hash = hash_password("original_password").unwrap();
    let user = TestUser {
        id: 0,
        username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        password: raw_hash.clone(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: chrono::Utc::now(),
        last_login: Some(chrono::Utc::now()),
    };
    let user = user.save(db).await.unwrap();

    let secret = b"my_password_reset_secret";
    let mail_backend = TestMailBackend::new();

    // 5. request_password_reset with an existing active user calls MailBackend::send with a message containing the token
    request_password_reset::<TestUser>(
        db,
        &mail_backend,
        "alice@example.com",
        secret,
        "https://example.com/reset/",
    )
    .await
    .unwrap();

    let sent = mail_backend.sent_messages.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, vec!["alice@example.com".to_string()]);
    assert!(sent[0].body.contains("https://example.com/reset/"));

    // Extract token from the email body
    let body = &sent[0].body;
    let url_start = body.find("https://example.com/reset/").unwrap();
    let token_start = url_start + "https://example.com/reset/".len();
    let token_end = body[token_start..]
        .find(|c: char| c.is_whitespace())
        .map(|idx| token_start + idx)
        .unwrap_or(body.len());
    let token = &body[token_start..token_end];

    // 6. request_password_reset with a nonexistent email does NOT call send, but still returns Ok(())
    let mail_backend_nonexistent = TestMailBackend::new();
    let res = request_password_reset::<TestUser>(
        db,
        &mail_backend_nonexistent,
        "nonexistent@example.com",
        secret,
        "https://example.com/reset/",
    )
    .await;
    assert!(res.is_ok());
    assert!(mail_backend_nonexistent
        .sent_messages
        .lock()
        .unwrap()
        .is_empty());

    // 7. confirm_password_reset with a valid token and new password actually changes the user's password
    confirm_password_reset::<TestUser>(db, user.id, token, "new_shiny_password", secret)
        .await
        .unwrap();

    // Fetch user from DB and verify password
    let updated_users = TestUser::objects()
        .filter(q!(id = user.id))
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert_eq!(updated_users.len(), 1);
    let updated_user = &updated_users[0];
    assert!(verify_password("new_shiny_password", updated_user.password_hash()).unwrap());
    assert!(!verify_password("original_password", updated_user.password_hash()).unwrap());

    // 8. confirm_password_reset with an invalid/expired token leaves the password unchanged and returns an error
    let invalid_token = "invalid.token.here";
    let res_invalid =
        confirm_password_reset::<TestUser>(db, user.id, invalid_token, "another_password", secret)
            .await;
    assert!(res_invalid.is_err());

    // Verify password is still "new_shiny_password"
    let final_users = TestUser::objects()
        .filter(q!(id = user.id))
        .unwrap()
        .all(db)
        .await
        .unwrap();
    let final_user = &final_users[0];
    assert!(verify_password("new_shiny_password", final_user.password_hash()).unwrap());

    // 9. confirm_password_reset with a weak password rejects with WeakPassword and leaves the old hash unchanged
    let fresh_token =
        generate_password_reset_token(final_user, secret, std::time::Duration::from_secs(3600));
    let weak_res =
        confirm_password_reset::<TestUser>(db, user.id, &fresh_token, "1234", secret).await;
    assert!(matches!(weak_res, Err(AuthError::WeakPassword(ref errs)) if !errs.is_empty()));
    let after_weak = TestUser::objects()
        .filter(q!(id = user.id))
        .unwrap()
        .all(db)
        .await
        .unwrap();
    assert!(verify_password("new_shiny_password", after_weak[0].password_hash()).unwrap());
    assert!(!verify_password("another_password", final_user.password_hash()).unwrap());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_permissions_and_groups() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    ensure_cross_process_lock_held().await;
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    // Drop tables if they exist (in reverse dependency order)
    let drop_tables = [
        "auth_user_permissions",
        "auth_group_permissions",
        "auth_user_groups",
        "auth_group",
        "auth_permission",
        "auth_user",
    ];
    for table in drop_tables {
        let sql = format!("DROP TABLE IF EXISTS {table}");
        let _ = db.conn().execute(&sql, &[]).await;
    }

    // Create tables
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

    let create_perm_sql = format!(
        "CREATE TABLE auth_permission (
            id {auto_pk},
            codename VARCHAR(255) NOT NULL UNIQUE,
            name VARCHAR(255) NOT NULL
        )"
    );
    db.conn().execute(&create_perm_sql, &[]).await.unwrap();

    let create_group_sql = format!(
        "CREATE TABLE auth_group (
            id {auto_pk},
            name VARCHAR(150) NOT NULL UNIQUE
        )"
    );
    db.conn().execute(&create_group_sql, &[]).await.unwrap();

    let create_user_groups_sql = format!(
        "CREATE TABLE auth_user_groups (
            id {auto_pk},
            \"user\" BIGINT NOT NULL,
            \"group\" BIGINT NOT NULL
        )"
    );
    db.conn()
        .execute(&create_user_groups_sql, &[])
        .await
        .unwrap();

    let create_group_perms_sql = format!(
        "CREATE TABLE auth_group_permissions (
            id {auto_pk},
            \"group\" BIGINT NOT NULL,
            permission BIGINT NOT NULL
        )"
    );
    db.conn()
        .execute(&create_group_perms_sql, &[])
        .await
        .unwrap();

    let create_user_perms_sql = format!(
        "CREATE TABLE auth_user_permissions (
            id {auto_pk},
            \"user\" BIGINT NOT NULL,
            permission BIGINT NOT NULL
        )"
    );
    db.conn()
        .execute(&create_user_perms_sql, &[])
        .await
        .unwrap();

    // 1. Call sync_permissions → expect exactly 4 permissions per registered model
    let models: Vec<_> = djangors_orm::meta::all_registered_models().collect();
    let expected_count = models.len() * 4;
    let synced = sync_permissions(db).await.unwrap();
    assert_eq!(synced, expected_count);

    let row_count = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM auth_permission", &[])
        .await
        .unwrap();
    let count_db = row_count.try_i64(0).unwrap().unwrap();
    assert_eq!(count_db, expected_count as i64);

    // Verify codenames conform to the convention
    let sample_codename = format!(
        "{}.view_{}",
        models[0].app_label,
        models[0].struct_name.to_lowercase()
    );
    let p1 = db.dialect().placeholder(1);
    let sql_exists = format!("SELECT COUNT(*) FROM auth_permission WHERE codename = {p1}");
    let row_exists = db
        .conn()
        .fetch_one(
            &sql_exists,
            &[djangors_db::BindValue::Text(sample_codename.clone())],
        )
        .await
        .unwrap();
    let codename_count = row_exists.try_i64(0).unwrap().unwrap();
    assert!(
        codename_count > 0,
        "Expected codename {} to exist",
        sample_codename
    );

    // 2. Call sync_permissions twice → idempotent, no error, same count
    let synced_again = sync_permissions(db).await.unwrap();
    assert_eq!(synced_again, expected_count);

    let row_count_again = db
        .conn()
        .fetch_one("SELECT COUNT(*) FROM auth_permission", &[])
        .await
        .unwrap();
    let count_db_again = row_count_again.try_i64(0).unwrap().unwrap();
    assert_eq!(count_db_again, expected_count as i64);

    // Create a test user in auth_user
    let now = chrono::Utc::now();
    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let p3 = db.dialect().placeholder(3);
    let p4 = db.dialect().placeholder(4);
    let p5 = db.dialect().placeholder(5);
    let p6 = db.dialect().placeholder(6);
    let p7 = db.dialect().placeholder(7);
    let insert_user_sql = format!(
        "INSERT INTO auth_user (username, email, password, is_active, is_staff, is_superuser, date_joined) \
         VALUES ({p1}, {p2}, {p3}, {p4}, {p5}, {p6}, {p7}) RETURNING id"
    );
    let row_u = db
        .conn()
        .fetch_one(
            &insert_user_sql,
            &[
                djangors_db::BindValue::Text("testpermuser".to_string()),
                djangors_db::BindValue::Text("test@example.com".to_string()),
                djangors_db::BindValue::Text("password_hash".to_string()),
                djangors_db::BindValue::Bool(true),
                djangors_db::BindValue::Bool(false),
                djangors_db::BindValue::Bool(false),
                djangors_db::BindValue::DateTime(now),
            ],
        )
        .await
        .unwrap();
    let user_id = row_u.try_i64(0).unwrap().unwrap();

    let check_codename = "djangors_auth.add_user";

    let sel_perm_sql = format!("SELECT id FROM auth_permission WHERE codename = {p1}");
    let row_p = db
        .conn()
        .fetch_one(
            &sel_perm_sql,
            &[djangors_db::BindValue::Text(check_codename.to_string())],
        )
        .await
        .unwrap();
    let perm_id = row_p.try_i64(0).unwrap().unwrap();

    // 3. has_perm false with no grants
    let has = has_perm(db, user_id, check_codename).await.unwrap();
    assert!(!has);

    // 4. has_perm true via direct UserPermission
    let insert_up_sql =
        format!("INSERT INTO auth_user_permissions (\"user\", permission) VALUES ({p1}, {p2})");
    db.conn()
        .execute(
            &insert_up_sql,
            &[
                djangors_db::BindValue::I64(user_id),
                djangors_db::BindValue::I64(perm_id),
            ],
        )
        .await
        .unwrap();
    let has = has_perm(db, user_id, check_codename).await.unwrap();
    assert!(has);

    // Clean up direct UserPermission for next tests
    let del_up_sql = format!("DELETE FROM auth_user_permissions WHERE \"user\" = {p1}");
    db.conn()
        .execute(&del_up_sql, &[djangors_db::BindValue::I64(user_id)])
        .await
        .unwrap();
    let has = has_perm(db, user_id, check_codename).await.unwrap();
    assert!(!has);

    // 5. has_perm true via group membership (this exercises join chain and quotes)
    let insert_group_sql = format!("INSERT INTO auth_group (name) VALUES ({p1}) RETURNING id");
    let row_g = db
        .conn()
        .fetch_one(
            &insert_group_sql,
            &[djangors_db::BindValue::Text("testgroup".to_string())],
        )
        .await
        .unwrap();
    let group_id = row_g.try_i64(0).unwrap().unwrap();

    let insert_ug_sql =
        format!("INSERT INTO auth_user_groups (\"user\", \"group\") VALUES ({p1}, {p2})");
    db.conn()
        .execute(
            &insert_ug_sql,
            &[
                djangors_db::BindValue::I64(user_id),
                djangors_db::BindValue::I64(group_id),
            ],
        )
        .await
        .unwrap();

    let insert_gp_sql =
        format!("INSERT INTO auth_group_permissions (\"group\", permission) VALUES ({p1}, {p2})");
    db.conn()
        .execute(
            &insert_gp_sql,
            &[
                djangors_db::BindValue::I64(group_id),
                djangors_db::BindValue::I64(perm_id),
            ],
        )
        .await
        .unwrap();

    let has = has_perm(db, user_id, check_codename).await.unwrap();
    assert!(has);

    // 6. has_perm false when checking a different non-matching permission
    let non_matching_codename = "djangors_auth.delete_user";
    let has_non_matching = has_perm(db, user_id, non_matching_codename).await.unwrap();
    assert!(!has_non_matching);

    // Clean up all tables
    for table in drop_tables {
        let sql = format!("DROP TABLE IF EXISTS {table}");
        let _ = db.conn().execute(&sql, &[]).await;
    }
}
