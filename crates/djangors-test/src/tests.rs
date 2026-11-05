use super::*;
use djangors_core::Response;

async fn get_handler(
    _: Request,
    _: djangors_core::PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    Ok(Response::text(StatusCode::OK, "hello from test"))
}

async fn post_handler(
    req: Request,
    _: djangors_core::PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    let session = req.ext::<Session>().expect("session missing");
    let body = String::from_utf8_lossy(req.body_bytes().await);
    Ok(Response::text(
        StatusCode::CREATED,
        &format!(
            "{}:{}",
            session.get::<String>("user").unwrap_or_default(),
            body
        ),
    ))
}

async fn json_handler(
    req: Request,
    _: djangors_core::PathParams,
) -> Result<Response, djangors_core::DjangorsError> {
    let authorization = req
        .header("authorization")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_slice(req.body_bytes().await)
        .map_err(|error| djangors_core::DjangorsError::Internal(error.to_string()))?;
    Response::json(
        StatusCode::OK,
        &serde_json::json!({"authorization": authorization, "payload": payload}),
    )
}

fn client() -> TestClient {
    TestClient::new(
        Router::new()
            .get("/hello", get_handler)
            .post("/submit", post_handler),
    )
}

#[tokio::test]
async fn get_and_post_helpers_work() {
    client()
        .get("/hello")
        .send()
        .await
        .assert_status(StatusCode::OK)
        .assert_contains("hello from test");
    let session = Session::new_empty();
    session.set("user", "Ada");
    client()
        .post_form("/submit", &[("name", "Grace Hopper")])
        .with_session(session)
        .send()
        .await
        .assert_status(StatusCode::CREATED)
        .assert_contains("Ada:name=Grace+Hopper");
}

#[tokio::test]
async fn json_method_headers_and_response_deserialization_work() {
    let client = TestClient::new(Router::new().route("/api", Method::PATCH, json_handler));
    let response = client
        .request(Method::PATCH, "/api")
        .json(&serde_json::json!({"enabled": true}))
        .bearer("test-token")
        .header("Origin", "https://a.test")
        .send()
        .await
        .assert_status(StatusCode::OK);
    let value = response.json_value().unwrap();
    assert_eq!(value["authorization"], "Bearer test-token");
    assert_eq!(value["payload"]["enabled"], true);
}

#[tokio::test]
async fn from_service_runs_tower_layers_and_json_helpers_work() {
    use hyper::http::{HeaderName, HeaderValue};
    use tower::ServiceBuilder;
    use tower_http::set_header::SetResponseHeaderLayer;

    let service = ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-test-stack"),
            HeaderValue::from_static("active"),
        ))
        .service(Router::new().post("/api", json_handler));
    let client = TestClient::from_service(service);
    let response = client
        .post_json("/api", &serde_json::json!({"ok": 1}))
        .bearer("stack-token")
        .send()
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(response.headers()["x-test-stack"], "active");
    let value: serde_json::Value = response.json().unwrap();
    assert_eq!(value["payload"]["ok"], 1);
}

#[tokio::test]
#[should_panic(expected = "expected response body to contain")]
async fn assert_contains_panics_on_mismatch() {
    client()
        .get("/hello")
        .send()
        .await
        .assert_contains("missing");
}

// --- Isolated database tests (require DATABASE_URL to be set) ---

#[tokio::test]
async fn isolated_databases_are_separate() {
    if TestBackend::current() == TestBackend::Sqlite {
        // Requires Postgres current_database() function and separate named database catalog.
        return;
    }

    let db1 = TestDatabase::isolated()
        .await
        .expect("db1 isolation failed");
    let db2 = TestDatabase::isolated()
        .await
        .expect("db2 isolation failed");

    let name1: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(db1.database().pool())
        .await
        .expect("query current_database from db1");
    let name2: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(db2.database().pool())
        .await
        .expect("query current_database from db2");

    assert_ne!(name1, name2, "isolated databases must have different names");

    db1.create_table("CREATE TABLE IF NOT EXISTS shared_test (id SERIAL PRIMARY KEY, val TEXT)")
        .await
        .expect("create table in db1");
    db2.create_table("CREATE TABLE IF NOT EXISTS shared_test (id SERIAL PRIMARY KEY, val TEXT)")
        .await
        .expect("create table in db2");

    sqlx::query("INSERT INTO shared_test (val) VALUES ($1)")
        .bind("from db1")
        .execute(db1.database().pool())
        .await
        .expect("insert into db1");
    sqlx::query("INSERT INTO shared_test (val) VALUES ($1)")
        .bind("from db2")
        .execute(db2.database().pool())
        .await
        .expect("insert into db2");

    let count1: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shared_test")
        .fetch_one(db1.database().pool())
        .await
        .expect("count in db1");
    let count2: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM shared_test")
        .fetch_one(db2.database().pool())
        .await
        .expect("count in db2");

    assert_eq!(count1, 1, "db1 must see exactly 1 row");
    assert_eq!(count2, 1, "db2 must see exactly 1 row");

    db1.cleanup().await.expect("cleanup db1");
    db2.cleanup().await.expect("cleanup db2");
}

#[tokio::test]
async fn cleanup_actually_drops_database() {
    if TestBackend::current() == TestBackend::Sqlite {
        // Requires Postgres catalog pg_database and server drop database functionality.
        return;
    }

    let db = TestDatabase::isolated()
        .await
        .expect("isolated for cleanup test");
    let db_name = db.db_name.as_ref().cloned().unwrap();
    let admin_base_url = db.admin_base_url.clone();

    let admin_url = replace_db_name(&admin_base_url, "postgres");
    let admin_db = Database::connect(&DatabaseConfig::new(admin_url))
        .await
        .expect("admin connect for cleanup pre-check");

    let exists_before: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&db_name)
            .fetch_one(admin_db.pool())
            .await
            .expect("check database exists before cleanup");
    assert!(exists_before, "database must exist before cleanup");

    drop(admin_db);
    db.cleanup().await.expect("cleanup should succeed");

    let admin_url = replace_db_name(&admin_base_url, "postgres");
    let admin_db = Database::connect(&DatabaseConfig::new(admin_url))
        .await
        .expect("admin connect for cleanup post-check");

    let exists_after: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&db_name)
            .fetch_one(admin_db.pool())
            .await
            .expect("check database exists after cleanup");
    assert!(!exists_after, "database must be gone after cleanup");
}

#[tokio::test]
async fn fixtures_loader_inserts_queryable_rows() {
    use djangors_orm::error::FromRow;
    use djangors_orm::expr::Value;
    use djangors_orm::{DefaultValue, FieldKind, FieldMeta, Model, ModelMeta, QuerySet};
    use serde::Deserialize;

    #[derive(Debug, Clone, Deserialize)]
    struct FixturesTestModel {
        #[serde(default)]
        id: i64,
        name: String,
        value: i64,
    }

    impl Model for FixturesTestModel {
        fn meta() -> &'static ModelMeta {
            use std::sync::OnceLock;
            static META: OnceLock<ModelMeta> = OnceLock::new();
            META.get_or_init(|| ModelMeta {
                struct_name: "FixturesTestModel",
                app_label: "test_app",
                table_name: "test_fixtures_model",
                fields: &[
                    FieldMeta {
                        name: "id",
                        column_name: "id",
                        kind: FieldKind::BigInt,
                        primary_key: true,
                        auto: true,
                        nullable: false,
                        unique: true,
                        db_index: false,
                        default: DefaultValue::None,
                        max_length: None,
                        verbose_name: None,
                        help_text: None,
                        choices: &[],
                    },
                    FieldMeta {
                        name: "name",
                        column_name: "name",
                        kind: FieldKind::Text,
                        primary_key: false,
                        auto: false,
                        nullable: false,
                        unique: false,
                        db_index: false,
                        default: DefaultValue::None,
                        max_length: None,
                        verbose_name: None,
                        help_text: None,
                        choices: &[],
                    },
                    FieldMeta {
                        name: "value",
                        column_name: "value",
                        kind: FieldKind::BigInt,
                        primary_key: false,
                        auto: false,
                        nullable: false,
                        unique: false,
                        db_index: false,
                        default: DefaultValue::None,
                        max_length: None,
                        verbose_name: None,
                        help_text: None,
                        choices: &[],
                    },
                ],
                relations: &[],
                indexes: &[],
                unique_together: &[],
                ordering: &[],
            })
        }

        fn field_values(&self) -> Vec<(&'static str, Value)> {
            vec![
                ("id", Value::I64(self.id)),
                ("name", Value::Text(self.name.clone())),
                ("value", Value::I64(self.value)),
            ]
        }

        fn field_names() -> Vec<&'static str> {
            vec!["id", "name", "value"]
        }
    }

    impl FromRow for FixturesTestModel {
        fn from_row(row: &djangors_orm::DbRow) -> Result<Self, djangors_orm::OrmError> {
            Ok(Self {
                id: row.try_i64_by_name("id")?.ok_or_else(|| {
                    djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound)
                })?,
                name: row.try_string_by_name("name")?.ok_or_else(|| {
                    djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound)
                })?,
                value: row.try_i64_by_name("value")?.ok_or_else(|| {
                    djangors_orm::OrmError::Query(djangors_orm::sqlx::Error::RowNotFound)
                })?,
            })
        }
    }

    let db = TestDatabase::isolated()
        .await
        .expect("isolated for fixtures test");

    let auto_pk = db.database().dialect().auto_pk_type();
    db.create_table(
            &format!("CREATE TABLE test_fixtures_model (id {auto_pk}, name TEXT NOT NULL, value BIGINT NOT NULL)"),
        )
        .await
        .expect("create fixtures table");

    let json = r#"[
            {"name": "Alice", "value": 100},
            {"name": "Bob", "value": 200}
        ]"#;

    let items: Vec<FixturesTestModel> = db
        .load_fixtures(json)
        .await
        .expect("load_fixtures should succeed");
    assert_eq!(items.len(), 2, "must deserialize 2 items");

    let queried: Vec<FixturesTestModel> =
        QuerySet::<FixturesTestModel>::all(&QuerySet::new(), db.database())
            .await
            .expect("query all from fixtures table");

    assert_eq!(queried.len(), 2, "must read back 2 rows");
    assert_eq!(queried[0].name, "Alice");
    assert_eq!(queried[0].value, 100);
    assert_eq!(queried[1].name, "Bob");
    assert_eq!(queried[1].value, 200);

    db.cleanup().await.expect("cleanup fixtures db");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn cross_process_lock_genuinely_serializes_two_separate_connections() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    // Two independent Database instances (their own separate connection
    // pools, matching two separate OS processes at the level that actually
    // matters here - Postgres session identity) racing for the SAME named
    // lock. A `tokio::sync::Barrier` forces both to request the lock at the
    // same instant, matching the rigor used elsewhere in this codebase to
    // rule out scheduling luck (see djangors-tasks' SKIP LOCKED
    // investigation) - not just "usually passes in practice".
    let db_a = Database::connect(&DatabaseConfig::new(url.clone()))
        .await
        .expect("connect db_a");
    let db_b = Database::connect(&DatabaseConfig::new(url))
        .await
        .expect("connect db_b");

    let lock_name = format!("test_cross_process_lock_{}", generate_db_name());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));

    let events = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<&'static str>::new()));

    let barrier_a = barrier.clone();
    let events_a = events.clone();
    let lock_name_a = lock_name.clone();
    let task_a = tokio::spawn(async move {
        barrier_a.wait().await;
        let lock = acquire_cross_process_lock(&db_a, &lock_name_a)
            .await
            .unwrap();
        events_a.lock().await.push("a_acquired");
        // Hold the lock long enough that, if the other connection were NOT
        // genuinely blocked, it would race in and acquire during this
        // window instead of waiting.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        events_a.lock().await.push("a_released");
        lock.release().await.unwrap();
    });

    let barrier_b = barrier.clone();
    let events_b = events.clone();
    let lock_name_b = lock_name.clone();
    let task_b = tokio::spawn(async move {
        barrier_b.wait().await;
        let lock = acquire_cross_process_lock(&db_b, &lock_name_b)
            .await
            .unwrap();
        events_b.lock().await.push("b_acquired");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        events_b.lock().await.push("b_released");
        lock.release().await.unwrap();
    });

    task_a.await.unwrap();
    task_b.await.unwrap();

    let log = events.lock().await;
    // Whichever of A/B actually wins the race for the lock is legitimately
    // non-deterministic (that's the point - both requested it at the same
    // instant), but real mutual exclusion means the two connections' held
    // periods can never overlap: the log must be exactly
    // [X_acquired, X_released, Y_acquired, Y_released], never anything
    // where the second acquire appears before the first release.
    assert_eq!(log.len(), 4, "log: {log:?}");
    let first = log[0].strip_suffix("_acquired").expect("log: {log:?}");
    assert_eq!(log[1], format!("{first}_released"), "log: {log:?}");
    let second = log[2].strip_suffix("_acquired").expect("log: {log:?}");
    assert_eq!(log[3], format!("{second}_released"), "log: {log:?}");
    assert_ne!(
        first, second,
        "log should show both connections, not the same one twice - log: {log:?}"
    );
}
