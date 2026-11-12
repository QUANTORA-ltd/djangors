use super::*;
use bytes::Bytes;
use djangors_core::error::DjangorsError;
use djangors_core::extract::FromRequest;
use djangors_core::request::Request;
use djangors_core::state::AppState;
use djangors_db::{Database, DatabaseConfig};
use djangors_orm::meta::Model;
use hyper::http::{HeaderMap, HeaderValue, Method, Uri};

async fn setup_db() -> Database {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    create_api_key_table(&db).await.unwrap();
    db
}

fn make_request(db: &Database, header_name: Option<(&str, &str)>) -> Request {
    let mut headers = HeaderMap::new();
    if let Some((name, val)) = header_name {
        headers.insert(
            hyper::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(val).unwrap(),
        );
    }
    Request::new(Method::GET, Uri::from_static("/"), headers, Bytes::new())
        .with_state(AppState::new().insert(db.clone()))
}

#[tokio::test]
async fn create_and_authenticate_with_x_api_key_and_authorization_header() {
    let db = setup_db().await;
    let (saved, full_key) = create_api_key(&db, "worker", &["read", "write"], None)
        .await
        .unwrap();

    let (prefix, secret) = full_key.split_once('.').unwrap();
    assert_eq!(prefix, saved.prefix);
    assert_ne!(saved.hashed_key, secret);
    assert_ne!(saved.hashed_key, full_key);
    assert!(saved.last_used_at.is_none());

    let req1 = make_request(&db, Some(("x-api-key", &full_key)));
    let ApiKeyAuth(authed1) = ApiKeyAuth::from_request(&req1).await.unwrap();
    assert_eq!(authed1.id, saved.id);
    assert!(authed1.last_used_at.is_some());

    let reloaded = ApiKey::objects()
        .filter(djangors_orm::q!(id = saved.id))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert!(reloaded.last_used_at.is_some());

    let auth_header = format!("Api-Key {full_key}");
    let req2 = make_request(&db, Some(("authorization", &auth_header)));
    let ApiKeyAuth(authed2) = ApiKeyAuth::from_request(&req2).await.unwrap();
    assert_eq!(authed2.prefix, saved.prefix);
}

#[tokio::test]
async fn invalid_revoked_and_expired_keys_return_same_unauthorized_message() {
    let db = setup_db().await;
    let (saved, full_key) = create_api_key(&db, "svc", &["read"], None).await.unwrap();
    let (prefix, _) = full_key.split_once('.').unwrap();

    // Wrong secret
    let wrong_secret = format!("{prefix}.{}", "0".repeat(64));
    let err_wrong_secret =
        ApiKeyAuth::from_request(&make_request(&db, Some(("x-api-key", &wrong_secret))))
            .await
            .unwrap_err();
    assert!(matches!(
        err_wrong_secret,
        DjangorsError::Unauthorized(ref msg) if msg == "invalid API key"
    ));

    // Unknown prefix
    let unknown_prefix = format!("unknown1.{}", "a".repeat(64));
    let err_unknown =
        ApiKeyAuth::from_request(&make_request(&db, Some(("x-api-key", &unknown_prefix))))
            .await
            .unwrap_err();
    assert!(matches!(
        err_unknown,
        DjangorsError::Unauthorized(ref msg) if msg == "invalid API key"
    ));

    // Revoked key
    assert!(revoke_api_key(&db, &saved.prefix).await.unwrap());
    let err_revoked = ApiKeyAuth::from_request(&make_request(&db, Some(("x-api-key", &full_key))))
        .await
        .unwrap_err();
    assert!(matches!(
        err_revoked,
        DjangorsError::Unauthorized(ref msg) if msg == "invalid API key"
    ));

    // Expired key
    let past = chrono::Utc::now() - chrono::Duration::minutes(5);
    let (_expired_row, expired_key) = create_api_key(&db, "expired", &["read"], Some(past))
        .await
        .unwrap();
    let err_expired =
        ApiKeyAuth::from_request(&make_request(&db, Some(("x-api-key", &expired_key))))
            .await
            .unwrap_err();
    assert!(matches!(
        err_expired,
        DjangorsError::Unauthorized(ref msg) if msg == "invalid API key"
    ));
}

#[tokio::test]
async fn scopes_and_permission_policies() {
    let db = setup_db().await;
    let (saved, full_key) = create_api_key(&db, "billing", &["invoices:read", "jobs:run"], None)
        .await
        .unwrap();

    assert!(saved.has_scope("invoices:read"));
    assert!(saved.has_scope("jobs:run"));
    assert!(!saved.has_scope("invoices:write"));
    assert!(!saved.has_scope(""));

    let req_valid = make_request(&db, Some(("x-api-key", &full_key)));
    assert!(HasApiKey.has_permission(&req_valid).await);
    assert!(
        HasApiKeyScope("invoices:read")
            .has_permission(&req_valid)
            .await
    );
    assert!(
        !HasApiKeyScope("invoices:write")
            .has_permission(&req_valid)
            .await
    );

    let req_missing = make_request(&db, None);
    assert!(!HasApiKey.has_permission(&req_missing).await);
    assert!(
        !HasApiKeyScope("invoices:read")
            .has_permission(&req_missing)
            .await
    );
}

#[test]
fn openapi_builder_adds_api_key_security_scheme() {
    let mut builder = OpenApiBuilder::new("Test API", "1.0.0");
    builder.api_key_security();
    let spec = builder.build();
    let scheme = &spec["components"]["securitySchemes"]["ApiKeyAuth"];
    assert_eq!(scheme["type"], "apiKey");
    assert_eq!(scheme["in"], "header");
    assert_eq!(scheme["name"], "X-Api-Key");
}
