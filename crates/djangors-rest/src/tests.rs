use super::*;
use bytes::Bytes;
use djangors_core::extract::FromRequest;
use djangors_core::Router;
use djangors_macros::Model as DeriveModel;
use djangors_orm::expr::{UnresolvedCompare, UnresolvedExpr, Value};
use djangors_orm::queryset::QuerySet;
use hyper::http::{header::CONTENT_TYPE, HeaderMap, Method, Uri};
use hyper::StatusCode;
use std::sync::Mutex;

static DB_MUTEX: Mutex<()> = Mutex::new(());

#[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "rest_test", table_name = "rest_test_category")]
pub struct TestCategory {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
}

#[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "rest_test", table_name = "rest_test_article")]
pub struct TestArticle {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub title: String,
    pub view_count: i64,
    pub is_published: bool,
    pub published_at: chrono::DateTime<chrono::Utc>,
    #[djangors(foreign_key(on_delete = "cascade"))]
    pub category: djangors_orm::ForeignKey<TestCategory>,
}

#[test]
fn test_serialization_round_trip() {
    let now = chrono::Utc::now();
    let article = TestArticle {
        id: 1,
        title: "Rust REST Framework".to_string(),
        view_count: 42,
        is_published: true,
        published_at: now,
        category: djangors_orm::ForeignKey::new(10),
    };

    let json = serialize(&article);
    assert_eq!(json["id"], 1);
    assert_eq!(json["title"], "Rust REST Framework");
    assert_eq!(json["view_count"], 42);
    assert_eq!(json["is_published"], true);
    assert_eq!(json["published_at"], now.to_rfc3339());
    assert_eq!(json["category"], 10);

    let deserialized_values = deserialize::<TestArticle>(&json).unwrap();
    assert_eq!(
        deserialized_values,
        vec![
            ("title", Value::Text("Rust REST Framework".to_string())),
            ("view_count", Value::I64(42)),
            ("is_published", Value::Bool(true)),
            ("published_at", Value::DateTime(now)),
            ("category", Value::I64(10)),
        ]
    );
}

#[test]
fn test_deserialization_multiple_errors() {
    let bad_json = serde_json::json!({
        "title": 12345, // invalid, must be string
        "view_count": "not_an_int", // invalid, must be integer
        "is_published": "maybe", // invalid, must be boolean
        "published_at": "invalid_date_string", // invalid datetime
        "category": "not_an_id" // invalid relation ID
    });

    let errs = deserialize::<TestArticle>(&bad_json).unwrap_err();
    assert!(errs.contains_key("title"));
    assert!(errs.contains_key("view_count"));
    assert!(errs.contains_key("is_published"));
    assert!(errs.contains_key("published_at"));
    assert!(errs.contains_key("category"));
    assert!(errs.get("title").unwrap().contains("must be a string"));
    assert!(errs.get("view_count").unwrap().contains("valid integer"));
}

#[test]
fn token_keys_are_high_entropy_and_fit_the_model() {
    let first = generate_token_key();
    let second = generate_token_key();
    assert_eq!(first.len(), 64);
    assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(first, second);
}

#[tokio::test]
async fn viewset_routes_require_authentication_by_default() {
    let router = viewset_routes::<TestArticle>(Router::new(), "/api/articles");
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert!(matches!(
        router.handle(req).await,
        Err(DjangorsError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn malformed_token_header_is_unauthorized() {
    let req = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert!(matches!(
        TokenAuth::from_request(&req).await,
        Err(DjangorsError::Unauthorized(_))
    ));
}

#[tokio::test]
async fn by_authenticated_user_rate_limit_key_rejects_unauthenticated_requests() {
    use djangors_core::RateLimitKey;
    let req = Request::new(
        Method::GET,
        Uri::from_static("/"),
        HeaderMap::new(),
        Bytes::new(),
    );
    let result = ByAuthenticatedUser.key(&req).await;
    assert!(matches!(result, Err(DjangorsError::Unauthorized(_))));
}

#[cfg(feature = "jwt")]
#[test]
fn jwt_round_trip_and_tamper_rejection() {
    let token = encode_jwt(42, "test-secret");
    assert_eq!(decode_jwt(&token, "test-secret").unwrap(), 42);
    assert!(decode_jwt(&format!("{token}tampered"), "test-secret").is_err());
    assert!(decode_jwt(&token, "wrong-secret").is_err());
}

#[cfg(feature = "jwt")]
#[test]
fn jwt_config_controls_ttl_and_leeway() {
    use std::time::Duration;

    #[derive(serde::Deserialize)]
    struct Claims {
        exp: u64,
    }

    let config = JwtConfig::new(Duration::from_secs(120)).with_leeway(Duration::from_secs(5));
    assert_eq!(config.access_ttl, Duration::from_secs(120));
    assert_eq!(config.leeway, Duration::from_secs(5));

    let before = jsonwebtoken::get_current_timestamp();
    let token = encode_jwt_with_config(7, "test-secret", &config);
    let claims = jsonwebtoken::dangerous::insecure_decode::<Claims>(&token)
        .unwrap()
        .claims;
    assert!(claims.exp >= before + 120);
    assert!(claims.exp <= jsonwebtoken::get_current_timestamp() + 120);
    assert_eq!(
        decode_jwt_with_config(&token, "test-secret", &config).unwrap(),
        7
    );

    // A token that expired an hour ago is rejected by the default leeway and
    // accepted only when the configured leeway covers the gap.
    let expired = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &serde_json::json!({
            "user_id": 7,
            "exp": jsonwebtoken::get_current_timestamp().saturating_sub(3600),
        }),
        &jsonwebtoken::EncodingKey::from_secret(b"test-secret"),
    )
    .unwrap();
    assert!(decode_jwt_with_config(&expired, "test-secret", &config).is_err());

    let generous = JwtConfig::new(Duration::from_secs(60)).with_leeway(Duration::from_secs(7200));
    assert_eq!(
        decode_jwt_with_config(&expired, "test-secret", &generous).unwrap(),
        7
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_viewset_crud_end_to_end() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_article", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_category", &[])
        .await;

    let create_cat_sql = format!(
        "CREATE TABLE rest_test_category (
                id {auto_pk},
                name VARCHAR(100) NOT NULL
            )"
    );
    db.conn().execute(&create_cat_sql, &[]).await.unwrap();

    let create_art_sql = format!(
        "CREATE TABLE rest_test_article (
                id {auto_pk},
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at {ts_type} NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )"
    );
    db.conn().execute(&create_art_sql, &[]).await.unwrap();

    let cat = TestCategory {
        id: 0,
        name: "Tech".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let router =
        viewset_routes_with_permission::<TestArticle, _>(Router::new(), "/api/articles", AllowAny)
            .with_state(db.clone());

    // 1. Create Article via POST (Validation Error test)
    let invalid_create_body = serde_json::json!({
        "view_count": "invalid",
        "category": cat.id
    });
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    let req = Request::new(
        Method::POST,
        Uri::from_static("/api/articles"),
        headers.clone(),
        Bytes::from(serde_json::to_vec(&invalid_create_body).unwrap()),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let err = router.handle(req).await.unwrap_err();
    assert_eq!(err.status_code(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(err.code(), VALIDATION_ERROR_CODE);
    let details = err.details().expect("validation errors carry a field map");
    assert!(
        details.get("title").is_some(),
        "missing required field must be reported against `title`, got {details}"
    );
    assert!(
        details.get("view_count").is_some(),
        "unparseable integer must be reported against `view_count`, got {details}"
    );

    // 2. Create Article via POST (Success)
    let valid_create_body = serde_json::json!({
        "title": "First Article",
        "view_count": 100,
        "is_published": true,
        "published_at": "2026-07-22T14:00:00Z",
        "category": cat.id
    });
    let req = Request::new(
        Method::POST,
        Uri::from_static("/api/articles"),
        headers.clone(),
        Bytes::from(serde_json::to_vec(&valid_create_body).unwrap()),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let created_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(created_json["id"], 1);
    assert_eq!(created_json["title"], "First Article");

    // 3. List Articles via GET
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(list_json["count"], 1);
    assert_eq!(list_json["page"], 1);
    assert_eq!(list_json["total_pages"], 1);
    assert_eq!(list_json["results"].as_array().unwrap().len(), 1);

    // 4. Retrieve Article via GET /{pk}
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles/1"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let retrieve_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(retrieve_json["id"], 1);

    // Retrieve 404 test
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles/999"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(matches!(res, Err(DjangorsError::NotFound)));

    // 5. Update Article via PUT /{pk}
    let update_body = serde_json::json!({
        "title": "Updated Article Title",
        "view_count": 200,
        "is_published": false,
        "published_at": "2026-07-22T15:00:00Z",
        "category": cat.id
    });
    let req = Request::new(
        Method::PUT,
        Uri::from_static("/api/articles/1"),
        headers.clone(),
        Bytes::from(serde_json::to_vec(&update_body).unwrap()),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let updated_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(updated_json["title"], "Updated Article Title");
    assert_eq!(updated_json["view_count"], 200);

    // 6. Destroy Article via DELETE /{pk}
    let req = Request::new(
        Method::DELETE,
        Uri::from_static("/api/articles/1"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::NO_CONTENT);

    // Confirm deleted
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles/1"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await;
    assert!(matches!(res, Err(DjangorsError::NotFound)));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_viewset_pagination() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_article", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_category", &[])
        .await;

    let create_cat_sql = format!(
        "CREATE TABLE rest_test_category (
                id {auto_pk},
                name VARCHAR(100) NOT NULL
            )"
    );
    db.conn().execute(&create_cat_sql, &[]).await.unwrap();

    let create_art_sql = format!(
        "CREATE TABLE rest_test_article (
                id {auto_pk},
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at {ts_type} NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )"
    );
    db.conn().execute(&create_art_sql, &[]).await.unwrap();

    let cat = TestCategory {
        id: 0,
        name: "Tech".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let now = chrono::Utc::now();
    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let p3 = db.dialect().placeholder(3);
    let p4 = db.dialect().placeholder(4);
    let p5 = db.dialect().placeholder(5);
    let ins_art_sql = format!(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ({p1}, {p2}, {p3}, {p4}, {p5})"
        );

    // Insert 105 articles to span 2 pages (REST_PER_PAGE = 100)
    for i in 1..=105 {
        db.conn()
            .execute(
                &ins_art_sql,
                &[
                    djangors_db::BindValue::Text(format!("Article {i}")),
                    djangors_db::BindValue::I64(i as i64),
                    djangors_db::BindValue::Bool(true),
                    djangors_db::BindValue::DateTime(now),
                    djangors_db::BindValue::I64(cat.id),
                ],
            )
            .await
            .unwrap();
    }

    let router =
        viewset_routes_with_permission::<TestArticle, _>(Router::new(), "/api/articles", AllowAny)
            .with_state(db.clone());

    // Page 1 (default page = 1)
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?page=1"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let page1_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(page1_json["count"], 105);
    assert_eq!(page1_json["page"], 1);
    assert_eq!(page1_json["total_pages"], 2);
    assert_eq!(page1_json["results"].as_array().unwrap().len(), 100);

    // Page 2
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?page=2"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let page2_json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(page2_json["count"], 105);
    assert_eq!(page2_json["page"], 2);
    assert_eq!(page2_json["total_pages"], 2);
    assert_eq!(page2_json["results"].as_array().unwrap().len(), 5);
}

#[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "rest_test", table_name = "rest_test_full_types")]
pub struct TestFullTypes {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 50)]
    pub title: String,
    pub description: String,
    pub view_count: i64,
    pub is_active: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub price: String, // DecimalField equivalent test
    #[djangors(foreign_key(on_delete = "cascade"))]
    pub category: djangors_orm::ForeignKey<TestCategory>,
}

#[test]
fn test_openapi_schema_generation() {
    let schema = openapi_schema_for::<TestArticle>();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["title"], "TestArticle");
    assert!(schema["properties"]["id"].is_object());
    assert_eq!(schema["properties"]["title"]["type"], "string");
    assert_eq!(schema["properties"]["view_count"]["type"], "integer");
    assert_eq!(schema["properties"]["is_published"]["type"], "boolean");
    assert_eq!(schema["properties"]["published_at"]["type"], "string");
    assert_eq!(schema["properties"]["published_at"]["format"], "date-time");
    assert_eq!(schema["properties"]["category"]["type"], "integer");
    assert_eq!(schema["properties"]["category"]["format"], "int64");

    let full_schema = openapi_schema_for::<TestFullTypes>();
    assert_eq!(full_schema["properties"]["title"]["maxLength"], 50);
}

#[test]
fn test_openapi_builder_build() {
    let mut builder = OpenApiBuilder::new("Test API", "1.2.3");
    builder.register::<TestArticle>("/api/articles");

    let doc = builder.build();
    assert_eq!(doc["openapi"], "3.1.0");
    assert_eq!(doc["info"]["title"], "Test API");
    assert_eq!(doc["info"]["version"], "1.2.3");

    let paths = &doc["paths"];
    assert!(paths.get("/api/articles").is_some());
    assert!(paths["/api/articles"]["get"].is_object());
    assert!(paths["/api/articles"]["post"].is_object());

    assert!(paths.get("/api/articles/{pk}").is_some());
    assert!(paths["/api/articles/{pk}"]["get"].is_object());
    assert!(paths["/api/articles/{pk}"]["put"].is_object());
    assert!(paths["/api/articles/{pk}"]["patch"].is_object());
    assert!(paths["/api/articles/{pk}"]["delete"].is_object());

    let schemas = &doc["components"]["schemas"];
    assert!(schemas.get("TestArticle").is_some());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_viewset_filtering_and_ordering_end_to_end() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_article", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_category", &[])
        .await;

    let create_cat_sql = format!(
        "CREATE TABLE rest_test_category (
                id {auto_pk},
                name VARCHAR(100) NOT NULL
            )"
    );
    db.conn().execute(&create_cat_sql, &[]).await.unwrap();

    let create_art_sql = format!(
        "CREATE TABLE rest_test_article (
                id {auto_pk},
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at {ts_type} NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )"
    );
    db.conn().execute(&create_art_sql, &[]).await.unwrap();

    let cat = TestCategory {
        id: 0,
        name: "Tech".to_string(),
    }
    .save(db)
    .await
    .unwrap();

    let now = chrono::Utc::now();

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let p3 = db.dialect().placeholder(3);
    let p4 = db.dialect().placeholder(4);
    let p5 = db.dialect().placeholder(5);
    let ins_art_sql = format!(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ({p1}, {p2}, {p3}, {p4}, {p5})"
        );

    // Seed 3 articles:
    // A: title="Alpha", view_count=30, is_published=true
    // B: title="Beta", view_count=10, is_published=true
    // C: title="Gamma", view_count=20, is_published=false
    db.conn()
        .execute(
            &ins_art_sql,
            &[
                djangors_db::BindValue::Text("Alpha".into()),
                djangors_db::BindValue::I64(30),
                djangors_db::BindValue::Bool(true),
                djangors_db::BindValue::DateTime(now),
                djangors_db::BindValue::I64(cat.id),
            ],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &ins_art_sql,
            &[
                djangors_db::BindValue::Text("Beta".into()),
                djangors_db::BindValue::I64(10),
                djangors_db::BindValue::Bool(true),
                djangors_db::BindValue::DateTime(now),
                djangors_db::BindValue::I64(cat.id),
            ],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &ins_art_sql,
            &[
                djangors_db::BindValue::Text("Gamma".into()),
                djangors_db::BindValue::I64(20),
                djangors_db::BindValue::Bool(false),
                djangors_db::BindValue::DateTime(now),
                djangors_db::BindValue::I64(cat.id),
            ],
        )
        .await
        .unwrap();

    let viewset_config = ViewSetConfig {
        filterable_fields: &["is_published", "title"],
        orderable_fields: &["view_count", "title"],
        ..Default::default()
    };

    let router = viewset_routes_with_config_and_permission::<TestArticle, _>(
        Router::new(),
        "/api/articles",
        viewset_config,
        AllowAny,
    )
    .with_state(db.clone());

    // 1. Filter by allowlisted boolean field `is_published=true` (should return 2 items: Alpha, Beta)
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?is_published=true"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(json["count"], 2);

    // 2. Filter by allowlisted field `is_published=false` (should return 1 item: Gamma)
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?is_published=false"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["results"][0]["title"], "Gamma");

    // 3. Query param NOT in allowlist (`view_count=30`) must be ignored (returns all 3 items)
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?view_count=30"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(json["count"], 3);

    // 4. Order ascending by allowlisted field `ordering=view_count` (Beta 10, Gamma 20, Alpha 30)
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?ordering=view_count"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    let items = json["results"].as_array().unwrap();
    assert_eq!(items[0]["title"], "Beta");
    assert_eq!(items[1]["title"], "Gamma");
    assert_eq!(items[2]["title"], "Alpha");

    // 5. Order descending by allowlisted field `ordering=-view_count` (Alpha 30, Gamma 20, Beta 10)
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?ordering=-view_count"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    let items = json["results"].as_array().unwrap();
    assert_eq!(items[0]["title"], "Alpha");
    assert_eq!(items[1]["title"], "Gamma");
    assert_eq!(items[2]["title"], "Beta");

    // 6. Non-allowlisted order parameter (`ordering=-published_at`) must be ignored
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?ordering=-published_at"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = router.handle(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(json["count"], 3);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_cursor_pagination_handles_duplicate_ordering_values() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_article", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_category", &[])
        .await;
    let create_cat_sql =
        format!("CREATE TABLE rest_test_category (id {auto_pk}, name TEXT NOT NULL)");
    db.conn().execute(&create_cat_sql, &[]).await.unwrap();

    let create_art_sql = format!(
        "CREATE TABLE rest_test_article (
                id {auto_pk},
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at {ts_type} NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )"
    );
    db.conn().execute(&create_art_sql, &[]).await.unwrap();
    let cat = TestCategory {
        id: 0,
        name: "Tech".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let now = chrono::Utc::now();

    // 105 rows, ALL sharing the same view_count
    let total_rows = 105;
    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let p3 = db.dialect().placeholder(3);
    let p4 = db.dialect().placeholder(4);
    let p5 = db.dialect().placeholder(5);
    let ins_art_sql = format!(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ({p1}, {p2}, {p3}, {p4}, {p5})"
        );
    for i in 0..total_rows {
        db.conn()
            .execute(
                &ins_art_sql,
                &[
                    djangors_db::BindValue::Text(format!("Row-{i}")),
                    djangors_db::BindValue::I64(10),
                    djangors_db::BindValue::Bool(true),
                    djangors_db::BindValue::DateTime(now),
                    djangors_db::BindValue::I64(cat.id),
                ],
            )
            .await
            .unwrap();
    }

    let viewset_config = ViewSetConfig {
        orderable_fields: &["view_count"],
        cursor_pagination: true,
        ..Default::default()
    };

    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?ordering=view_count"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res1 = ViewSet::<TestArticle>::list_with_config(req1, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    assert_eq!(res1.status(), StatusCode::OK);
    let json1: serde_json::Value = serde_json::from_slice(res1.body()).unwrap();
    let page1_titles: Vec<String> = json1["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(page1_titles.len(), 100);
    let next_cursor = json1["next_cursor"].as_str().unwrap().to_string();

    let req2 = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/articles?ordering=view_count&cursor={next_cursor}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res2 = ViewSet::<TestArticle>::list_with_config(req2, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    assert_eq!(res2.status(), StatusCode::OK);
    let json2: serde_json::Value = serde_json::from_slice(res2.body()).unwrap();
    let page2_titles: Vec<String> = json2["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(page2_titles.len(), 5);

    let mut all_titles: Vec<String> = page1_titles.into_iter().chain(page2_titles).collect();
    all_titles.sort();
    all_titles.dedup();
    assert_eq!(
        all_titles.len(),
        total_rows,
        "every row must appear exactly once across both pages, no skips or duplicates"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_cursor_pagination_stable_under_concurrent_insert() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_article", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_category", &[])
        .await;
    let create_cat_sql =
        format!("CREATE TABLE rest_test_category (id {auto_pk}, name TEXT NOT NULL)");
    db.conn().execute(&create_cat_sql, &[]).await.unwrap();

    let create_art_sql = format!(
        "CREATE TABLE rest_test_article (
                id {auto_pk},
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at {ts_type} NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )"
    );
    db.conn().execute(&create_art_sql, &[]).await.unwrap();
    let cat = TestCategory {
        id: 0,
        name: "Tech".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    let now = chrono::Utc::now();

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let p3 = db.dialect().placeholder(3);
    let p4 = db.dialect().placeholder(4);
    let p5 = db.dialect().placeholder(5);
    let ins_art_sql = format!(
            "INSERT INTO rest_test_article (title, view_count, is_published, published_at, category) VALUES ({p1}, {p2}, {p3}, {p4}, {p5})"
        );

    // 102 rows with distinct, ascending view_count values 1..=102.
    for i in 1..=102_i64 {
        db.conn()
            .execute(
                &ins_art_sql,
                &[
                    djangors_db::BindValue::Text(format!("Row-{i}")),
                    djangors_db::BindValue::I64(i),
                    djangors_db::BindValue::Bool(true),
                    djangors_db::BindValue::DateTime(now),
                    djangors_db::BindValue::I64(cat.id),
                ],
            )
            .await
            .unwrap();
    }

    let viewset_config = ViewSetConfig {
        orderable_fields: &["view_count"],
        cursor_pagination: true,
        ..Default::default()
    };

    // Page 1: view_count 1..=100.
    let req1 = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?ordering=view_count"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res1 = ViewSet::<TestArticle>::list_with_config(req1, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    let json1: serde_json::Value = serde_json::from_slice(res1.body()).unwrap();
    let next_cursor = json1["next_cursor"].as_str().unwrap().to_string();

    // Concurrent write: insert a new row that sorts BEFORE the cursor position
    // (view_count = 50, well within page 1's already-served range).
    db.conn()
        .execute(
            &ins_art_sql,
            &[
                djangors_db::BindValue::Text("Inserted-After-Page-1".into()),
                djangors_db::BindValue::I64(50),
                djangors_db::BindValue::Bool(true),
                djangors_db::BindValue::DateTime(now),
                djangors_db::BindValue::I64(cat.id),
            ],
        )
        .await
        .unwrap();

    // Page 2 via the cursor from page 1: must be exactly the original 2 remaining rows
    // (view_count 101, 102) - the new row must not leak in (it sorts before the cursor
    // position), and neither of the 2 tail rows may be skipped or duplicated.
    let req2 = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/articles?ordering=view_count&cursor={next_cursor}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res2 = ViewSet::<TestArticle>::list_with_config(req2, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    let json2: serde_json::Value = serde_json::from_slice(res2.body()).unwrap();
    let page2_titles: Vec<&str> = json2["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["title"].as_str().unwrap())
        .collect();
    assert_eq!(page2_titles, vec!["Row-101", "Row-102"]);
    assert!(!page2_titles.contains(&"Inserted-After-Page-1"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_cursor_pagination_rejects_malformed_cursor_and_non_allowlisted_field() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_article", &[])
        .await;
    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_category", &[])
        .await;
    let create_cat_sql =
        format!("CREATE TABLE rest_test_category (id {auto_pk}, name TEXT NOT NULL)");
    db.conn().execute(&create_cat_sql, &[]).await.unwrap();

    let create_art_sql = format!(
        "CREATE TABLE rest_test_article (
                id {auto_pk},
                title VARCHAR(200) NOT NULL,
                view_count BIGINT NOT NULL,
                is_published BOOLEAN NOT NULL,
                published_at {ts_type} NOT NULL,
                category BIGINT NOT NULL REFERENCES rest_test_category(id) ON DELETE CASCADE
            )"
    );
    db.conn().execute(&create_art_sql, &[]).await.unwrap();
    let cat = TestCategory {
        id: 0,
        name: "Tech".to_string(),
    }
    .save(db)
    .await
    .unwrap();
    TestArticle {
        id: 0,
        title: "Solo".to_string(),
        view_count: 1,
        is_published: true,
        published_at: chrono::Utc::now(),
        category: djangors_orm::ForeignKey::new(cat.id),
    }
    .save(db)
    .await
    .unwrap();

    let viewset_config = ViewSetConfig {
        orderable_fields: &["view_count"],
        cursor_pagination: true,
        ..Default::default()
    };

    // Not valid base64 at all.
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/articles?ordering=view_count&cursor=%25%25not-base64%25%25"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res =
        ViewSet::<TestArticle>::list_with_config(req, PathParams::new(), &viewset_config).await;
    assert!(matches!(res, Err(DjangorsError::BadRequest(_))));

    // Valid base64, but no `|` separator at all (wrong format).
    let bad_format = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        "no-separator-here",
    );
    let req = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/articles?ordering=view_count&cursor={bad_format}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res =
        ViewSet::<TestArticle>::list_with_config(req, PathParams::new(), &viewset_config).await;
    assert!(matches!(res, Err(DjangorsError::BadRequest(_))));

    // Valid base64, correct `pk|value` shape, but the pk portion isn't numeric.
    let bad_pk = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "abc|5");
    let req = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/articles?ordering=view_count&cursor={bad_pk}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res =
        ViewSet::<TestArticle>::list_with_config(req, PathParams::new(), &viewset_config).await;
    assert!(matches!(res, Err(DjangorsError::BadRequest(_))));

    // A syntactically valid cursor, but `?ordering=` names a field NOT in orderable_fields
    let valid_cursor = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "1|1");
    let req = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/articles?ordering=title&cursor={valid_cursor}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res =
        ViewSet::<TestArticle>::list_with_config(req, PathParams::new(), &viewset_config).await;
    assert!(matches!(res, Err(DjangorsError::BadRequest(_))));

    // No `?ordering=` at all: cursor pagination must also be rejected, not silently ignored.
    let req = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/articles?cursor={valid_cursor}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res =
        ViewSet::<TestArticle>::list_with_config(req, PathParams::new(), &viewset_config).await;
    assert!(matches!(res, Err(DjangorsError::BadRequest(_))));
}

/// A per-request "current owner" marker
#[derive(Clone, Copy)]
struct CurrentOwner(i64);

#[derive(DeriveModel, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "rest_test", table_name = "rest_test_note")]
pub struct TestNote {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub owner_id: i64,
    pub body: String,
}

impl Scoped for TestNote {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        let owner = req
            .state::<CurrentOwner>()
            .ok_or_else(|| DjangorsError::Unauthorized("no current owner in request".into()))?;
        qs.filter(UnresolvedExpr::And(vec![UnresolvedCompare {
            field: "owner_id",
            value: Value::I64(owner.0),
        }]))
        .map_err(|e| DjangorsError::Internal(e.to_string()))
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_scoped_viewset_enforces_owner_isolation_end_to_end() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_note", &[])
        .await;

    let create_note_sql = format!(
        "CREATE TABLE rest_test_note (
                id {auto_pk},
                owner_id BIGINT NOT NULL,
                body VARCHAR(200) NOT NULL
            )"
    );
    db.conn().execute(&create_note_sql, &[]).await.unwrap();

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let ins_note_sql = format!("INSERT INTO rest_test_note (owner_id, body) VALUES ({p1}, {p2})");

    // Seed two tenants' rows directly (owner 1: two notes, owner 2: one note).
    db.conn()
        .execute(
            &ins_note_sql,
            &[
                djangors_db::BindValue::I64(1),
                djangors_db::BindValue::Text("owner1-note-a".into()),
            ],
        )
        .await
        .unwrap();
    db.conn()
        .execute(
            &ins_note_sql,
            &[
                djangors_db::BindValue::I64(1),
                djangors_db::BindValue::Text("owner1-note-b".into()),
            ],
        )
        .await
        .unwrap();
    db.conn()
        .execute(
            &ins_note_sql,
            &[
                djangors_db::BindValue::I64(2),
                djangors_db::BindValue::Text("owner2-note-a".into()),
            ],
        )
        .await
        .unwrap();

    // Owner 1 lists notes: must see exactly their own 2 rows, never owner 2's row.
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(1)),
    );
    let res = ScopedViewSet::<TestNote>::list(req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(json["count"], 2);
    let bodies: Vec<&str> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["body"].as_str().unwrap())
        .collect();
    assert!(bodies.contains(&"owner1-note-a"));
    assert!(bodies.contains(&"owner1-note-b"));
    assert!(!bodies.contains(&"owner2-note-a"));

    // Owner 2 lists notes: must see only their own 1 row.
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(2)),
    );
    let res = ScopedViewSet::<TestNote>::list(req, PathParams::new())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["results"][0]["body"], "owner2-note-a");

    // Owner 2 tries to retrieve one of owner 1's rows by primary key directly: must be
    // treated as not found, never leaked, even though the row genuinely exists in the table.
    let row = db
        .conn()
        .fetch_one(
            "SELECT id FROM rest_test_note WHERE body = 'owner1-note-a'",
            &[],
        )
        .await
        .unwrap();
    let owner1_note_a_id = row.try_i64(0).unwrap().unwrap();
    let mut params = PathParams::new();
    params.insert("pk", &owner1_note_a_id.to_string());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes/x"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(2)),
    );
    let res = ScopedViewSet::<TestNote>::retrieve(req, params).await;
    assert!(matches!(res, Err(DjangorsError::NotFound)));

    // Owner 1 CAN retrieve their own row by the same primary key.
    let mut params = PathParams::new();
    params.insert("pk", &owner1_note_a_id.to_string());
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes/x"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(1)),
    );
    let res = ScopedViewSet::<TestNote>::retrieve(req, params)
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // A request with no `CurrentOwner` in state at all (unauthenticated/unscoped caller):
    // `scope` itself must reject it rather than silently falling back to an unscoped queryset.
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(djangors_core::state::AppState::new().insert(db.clone()));
    let res = ScopedViewSet::<TestNote>::list(req, PathParams::new()).await;
    assert!(matches!(res, Err(DjangorsError::Unauthorized(_))));
}

#[tokio::test]
async fn scoped_viewset_routes_with_config_requires_authentication_on_list() {
    // Regression test: `scoped_viewset_routes_with_config`'s list handler
    // used to skip the permission check that create/retrieve/update/destroy
    // already got on the same router, so an unauthenticated `GET /` reached
    // `Scoped::scope` instead of being rejected up front. Every route this
    // helper mounts — including list — must require authentication before
    // running.
    let router = scoped_viewset_routes_with_config::<TestNote>(
        Router::new(),
        "/api/notes",
        ViewSetConfig::default(),
    );
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert!(matches!(
        router.handle(req).await,
        Err(DjangorsError::Unauthorized(_))
    ));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_scoped_viewset_cursor_pagination_preserves_isolation_across_pages() {
    let _guard = DB_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let test_db = djangors_test::TestDatabase::connect().await.unwrap();
    let db = test_db.database();
    let auto_pk = db.dialect().auto_pk_type();

    let _ = db
        .conn()
        .execute("DROP TABLE IF EXISTS rest_test_note", &[])
        .await;
    let create_note_sql = format!(
        "CREATE TABLE rest_test_note (
                id {auto_pk},
                owner_id BIGINT NOT NULL,
                body VARCHAR(200) NOT NULL
            )"
    );
    db.conn().execute(&create_note_sql, &[]).await.unwrap();

    let p1 = db.dialect().placeholder(1);
    let p2 = db.dialect().placeholder(2);
    let ins_note_sql = format!("INSERT INTO rest_test_note (owner_id, body) VALUES ({p1}, {p2})");

    // Owner 1 gets 105 rows, owner 2 gets 5 rows.
    for i in 0..105 {
        db.conn()
            .execute(
                &ins_note_sql,
                &[
                    djangors_db::BindValue::I64(1),
                    djangors_db::BindValue::Text(format!("owner1-note-{i}")),
                ],
            )
            .await
            .unwrap();
    }
    for i in 0..5 {
        db.conn()
            .execute(
                &ins_note_sql,
                &[
                    djangors_db::BindValue::I64(2),
                    djangors_db::BindValue::Text(format!("owner2-note-{i}")),
                ],
            )
            .await
            .unwrap();
    }

    let viewset_config = ViewSetConfig {
        orderable_fields: &["id"],
        cursor_pagination: true,
        ..Default::default()
    };

    // Owner 1, page 1: 100 rows, all owner1's.
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes?ordering=id"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(1)),
    );
    let res = ScopedViewSet::<TestNote>::list_with_config(req, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    let page1: Vec<String> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["body"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(page1.len(), 100);
    assert!(page1.iter().all(|b| b.starts_with("owner1-note-")));
    let next_cursor = json["next_cursor"].as_str().unwrap().to_string();

    // Owner 1, page 2 via cursor: the remaining 5 rows, still all owner1's
    let req = Request::new(
        Method::GET,
        Uri::from_static(Box::leak(
            format!("/api/notes?ordering=id&cursor={next_cursor}").into_boxed_str(),
        )),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(1)),
    );
    let res = ScopedViewSet::<TestNote>::list_with_config(req, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    let page2: Vec<String> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["body"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(page2.len(), 5);
    assert!(page2.iter().all(|b| b.starts_with("owner1-note-")));
    assert_eq!(json["next_cursor"], serde_json::Value::Null);

    let mut all: Vec<String> = page1.into_iter().chain(page2).collect();
    all.sort();
    all.dedup();
    assert_eq!(
        all.len(),
        105,
        "owner1 must see all 105 of their own rows exactly once"
    );

    // Owner 2: must only ever see their own 5 rows
    let req = Request::new(
        Method::GET,
        Uri::from_static("/api/notes?ordering=id"),
        HeaderMap::new(),
        Bytes::new(),
    )
    .with_state(
        djangors_core::state::AppState::new()
            .insert(db.clone())
            .insert(CurrentOwner(2)),
    );
    let res = ScopedViewSet::<TestNote>::list_with_config(req, PathParams::new(), &viewset_config)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(res.body()).unwrap();
    let owner2_notes: Vec<String> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["body"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(owner2_notes.len(), 5);
    assert!(owner2_notes.iter().all(|b| b.starts_with("owner2-note-")));
    assert_eq!(json["next_cursor"], serde_json::Value::Null);
}
