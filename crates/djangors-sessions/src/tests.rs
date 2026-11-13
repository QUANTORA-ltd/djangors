use super::*;
use bytes::Bytes;
use djangors_core::{DjangorsError, Request, Response, Router};
use http_body_util::Full;
use hyper::StatusCode;
use tower::ServiceExt;

#[test]
fn test_round_trip() {
    let secret = b"my-very-secret-key-that-is-long-enough";
    let store = SignedCookieStore::new(secret);
    let session = Session::new_empty();
    session.set("key_one", "hello_world".to_string());
    session.set("key_two", 42);

    let cookie_val = store.encode(&session);
    let decoded = store
        .decode(&cookie_val)
        .expect("Should decode successfully");

    assert_eq!(
        decoded.get::<String>("key_one"),
        Some("hello_world".to_string())
    );
    assert_eq!(decoded.get::<i32>("key_two"), Some(42));
}

#[test]
fn test_tamper_rejection() {
    let secret = b"my-very-secret-key-that-is-long-enough";
    let store = SignedCookieStore::new(secret);
    let session = Session::new_empty();
    session.set("secured_val", "confidential".to_string());

    let cookie_val = store.encode(&session);

    // Attempting to decode the valid cookie works
    let decoded_ok = store.decode(&cookie_val).expect("Should decode");
    assert_eq!(
        decoded_ok.get::<String>("secured_val"),
        Some("confidential".to_string())
    );

    // Tamper with the cookie string. A valid cookie is base64.base64.base64.
    // We will flip a byte in the MAC signature part (the last segment).
    let parts: Vec<&str> = cookie_val.split('.').collect();
    assert_eq!(parts.len(), 3);

    let tampered_mac = {
        let mut mac_bytes = base64::engine::general_purpose::STANDARD
            .decode(parts[2])
            .unwrap();
        // Flip the last byte to corrupt the MAC without making base64 malformed
        if let Some(last) = mac_bytes.last_mut() {
            *last ^= 0xFF;
        }
        base64::engine::general_purpose::STANDARD.encode(&mac_bytes)
    };

    let tampered_cookie = format!("{}.{}.{}", parts[0], parts[1], tampered_mac);
    let decoded_tampered = store.decode(&tampered_cookie);
    assert!(
        decoded_tampered.is_none(),
        "Tampered session must return None"
    );
}

#[test]
fn test_expiry() {
    let secret = b"my-very-secret-key-that-is-long-enough";
    // Create store with zero max-age
    let store = SignedCookieStore::new(secret).with_max_age(Duration::ZERO);
    let session = Session::new_empty();
    session.set("test", "value".to_string());

    let cookie_val = store.encode(&session);

    // Even though the signature is valid, decoding should fail immediately due to expiration check
    let decoded = store.decode(&cookie_val);
    assert!(decoded.is_none(), "Expired session must fail to decode");
}

#[test]
fn test_cycle_key() {
    let secret = b"my-very-secret-key-that-is-long-enough";
    let store = SignedCookieStore::new(secret);
    let session = Session::new_empty();
    session.set("foo", "bar".to_string());

    let cookie_val_1 = store.encode(&session);

    // Rotate session identity
    session.cycle_key();
    let cookie_val_2 = store.encode(&session);

    // Prove pre-rotation and post-rotation cookie strings are different
    assert_ne!(cookie_val_1, cookie_val_2);

    // Decode both and show that they both can get the value, but they have different session keys
    let decoded_1 = store.decode(&cookie_val_1).unwrap();
    let decoded_2 = store.decode(&cookie_val_2).unwrap();

    assert_eq!(decoded_1.get::<String>("foo"), Some("bar".to_string()));
    assert_eq!(decoded_2.get::<String>("foo"), Some("bar".to_string()));

    let sk_1 = decoded_1.get::<String>("_session_key").unwrap();
    let sk_2 = decoded_2.get::<String>("_session_key").unwrap();
    assert_ne!(sk_1, sk_2, "Session keys must be rotated/different");
}

#[test]
fn test_no_session_cookie_empty() {
    let secret = b"my-very-secret-key-that-is-long-enough";
    let store = SignedCookieStore::new(secret);
    let decoded = store.decode("");
    assert!(decoded.is_none());
}

// End-to-end tower/router integration tests
async fn set_handler(
    req: Request,
    _: djangors_core::PathParams,
) -> Result<Response, DjangorsError> {
    let session = req
        .ext::<Session>()
        .expect("Session extension must be present");
    let count: i32 = session.get("count").unwrap_or(0);
    session.set("count", count + 1);
    Ok(Response::text(StatusCode::OK, &(count + 1).to_string()))
}

async fn get_handler(
    req: Request,
    _: djangors_core::PathParams,
) -> Result<Response, DjangorsError> {
    let session = req
        .ext::<Session>()
        .expect("Session extension must be present");
    let count: i32 = session.get("count").unwrap_or(0);
    Ok(Response::text(StatusCode::OK, &count.to_string()))
}

#[tokio::test]
async fn test_end_to_end_router_session() {
    use tower::service_fn;
    use tower::ServiceBuilder;

    let router = Router::new()
        .post("/set", set_handler)
        .get("/get", get_handler);

    let secret = b"super-secret-key-for-testing-purposes-only";
    let store = SignedCookieStore::new(secret);
    let layer = SessionLayer::new(store);

    let svc_fn = service_fn(move |req: hyper::Request<Full<Bytes>>| {
        let router = router.clone();
        async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
    });

    let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

    // 1. Request with no session cookie (empty count) -> should return 1
    let req1 = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/set")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);

    // We must have received a Set-Cookie header
    let set_cookie_header = resp1.headers().get(SET_COOKIE).expect("Should set cookie");
    let set_cookie_str = set_cookie_header.to_str().unwrap().to_string();

    // Read response body
    use http_body_util::BodyExt;
    let body1 = resp1.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body1[..], b"1");

    // Extract the value from the Set-Cookie string (e.g. djangors_sessionid=VALUE; Path=...)
    let cookie_value = set_cookie_str
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_string();

    // 2. Request 2: carrying the cookie to /get
    let req2 = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri("/get")
        .header(COOKIE, format!("djangors_sessionid={}", cookie_value))
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body2[..], b"1");

    // 3. Request 3: calling /set again carrying the cookie -> should return 2
    let req3 = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/set")
        .header(COOKIE, format!("djangors_sessionid={}", cookie_value))
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp3 = svc.ready().await.unwrap().call(req3).await.unwrap();
    assert_eq!(resp3.status(), StatusCode::OK);
    let body3 = resp3.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body3[..], b"2");
}

#[tokio::test]
async fn test_no_session_cookie_handler_access() {
    use tower::service_fn;
    use tower::ServiceBuilder;

    let router = Router::new().get("/get", get_handler);
    let secret = b"super-secret-key-for-testing-purposes-only";
    let store = SignedCookieStore::new(secret);
    let layer = SessionLayer::new(store);

    let svc_fn = service_fn(move |req: hyper::Request<Full<Bytes>>| {
        let router = router.clone();
        async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
    });

    let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

    let req = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri("/get")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    use http_body_util::BodyExt;
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"0");
}

#[tokio::test]
async fn test_session_cookie_secure() {
    use tower::service_fn;
    use tower::ServiceBuilder;

    let router = Router::new().post("/set", set_handler);
    let secret = b"super-secret-key-for-testing-purposes-only";

    // 1. Default (secure not called or false)
    let store_default = SignedCookieStore::new(secret);
    let layer_default = SessionLayer::new(store_default);
    let svc_fn1 = service_fn({
        let router = router.clone();
        move |req| {
            let router = router.clone();
            async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
        }
    });
    let mut svc_default = ServiceBuilder::new().layer(layer_default).service(svc_fn1);

    let req1 = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/set")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp1 = svc_default.ready().await.unwrap().call(req1).await.unwrap();
    let set_cookie_default = resp1.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(!set_cookie_default.contains("; Secure"));

    // 2. with_secure(true)
    let store_secure = SignedCookieStore::new(secret).with_secure(true);
    let layer_secure = SessionLayer::new(store_secure);
    let svc_fn2 = service_fn({
        let router = router.clone();
        move |req| {
            let router = router.clone();
            async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
        }
    });
    let mut svc_secure = ServiceBuilder::new().layer(layer_secure).service(svc_fn2);

    let req2 = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/set")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp2 = svc_secure.ready().await.unwrap().call(req2).await.unwrap();
    let set_cookie_secure = resp2.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie_secure.contains("; Secure"));
}

#[tokio::test]
async fn test_per_session_expiry_cookie_attributes_and_server_enforcement() {
    use tower::{service_fn, Service, ServiceBuilder};

    let secret = b"my-very-secret-key-that-is-long-enough";
    let store = SignedCookieStore::new(secret);

    let router = Router::new()
        .post(
            "/browser",
            |req: Request, _: djangors_core::PathParams| async move {
                let sess = req.ext::<Session>().unwrap();
                sess.set("user", "alice");
                sess.set_expiry(None);
                Ok::<_, DjangorsError>(Response::text(StatusCode::OK, "ok"))
            },
        )
        .post(
            "/remember",
            |req: Request, _: djangors_core::PathParams| async move {
                let sess = req.ext::<Session>().unwrap();
                sess.set("user", "bob");
                sess.set_expiry(Some(Duration::from_secs(2_592_000)));
                Ok::<_, DjangorsError>(Response::text(StatusCode::OK, "ok"))
            },
        );

    let layer = SessionLayer::new(store);
    let svc_fn = service_fn({
        let router = router.clone();
        move |req| {
            let router = router.clone();
            async move { Ok::<_, std::convert::Infallible>(router.dispatch(req).await) }
        }
    });
    let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

    // Browser session omits Max-Age
    let req1 = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/browser")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
    let cookie1 = resp1.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(!cookie1.contains("Max-Age="));

    // Remember-me session sets Max-Age=2592000
    let req2 = hyper::Request::builder()
        .method(hyper::Method::POST)
        .uri("/remember")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
    let cookie2 = resp2.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(cookie2.contains("Max-Age=2592000"));

    // Server-side expiry enforcement on SignedCookieStore
    let zero_store = SignedCookieStore::new(secret);
    let sess = Session::new_empty();
    sess.set("k", "v");
    sess.set_expiry(Some(Duration::from_secs(0)));
    let encoded = zero_store.encode(&sess);
    assert!(zero_store.decode(&encoded).is_none());
}

#[tokio::test]
async fn test_database_store_lifecycle_and_for_user() {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = djangors_db::Database::connect(&config).await.unwrap();
    let db_handle = db.clone();
    let store = DatabaseStore::new(db);

    let sess = Session::new_empty();
    sess.set("_auth_user_id", 42_i64);
    sess.set("role", "admin");
    let ctx = SessionContext {
        ip: Some("127.0.0.1".to_string()),
        user_agent: Some("TestAgent/1.0".to_string()),
    };

    let raw_key = store.save(&sess, &ctx).await.unwrap();
    assert_eq!(raw_key.len(), 64);

    let loaded = store
        .load(&raw_key)
        .await
        .unwrap()
        .expect("session should load");
    assert_eq!(loaded.get::<i64>("_auth_user_id"), Some(42));
    assert_eq!(loaded.get::<String>("role"), Some("admin".to_string()));
    assert_eq!(loaded.session_key(), raw_key);

    // Only the hash may reach the database; the raw key must not appear anywhere in the row.
    let row = db_handle
        .conn()
        .fetch_one("SELECT key, data FROM djangors_session", &[])
        .await
        .unwrap();
    let stored_key = row.try_string(0).unwrap().unwrap();
    let stored_data = row.try_string(1).unwrap().unwrap();
    assert_eq!(stored_key, hash_session_key(&raw_key));
    assert!(
        !stored_data.contains(&raw_key),
        "raw session key leaked into data column"
    );

    let records = store.for_user(42).await.unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].key_hash, hash_session_key(&raw_key));
    assert_eq!(records[0].ip.as_deref(), Some("127.0.0.1"));
    assert_eq!(records[0].user_agent.as_deref(), Some("TestAgent/1.0"));

    // Delete by key_hash
    store.delete(&records[0].key_hash).await.unwrap();
    assert!(store.load(&raw_key).await.unwrap().is_none());
    assert!(store.for_user(42).await.unwrap().is_empty());

    // Expired session is rejected on server and cleaned up by clear_expired
    let expired_sess = Session::new_empty();
    expired_sess.set("_auth_user_id", 99_i64);
    expired_sess.set_expiry(Some(Duration::from_secs(0)));
    let expired_key = store.save(&expired_sess, &ctx).await.unwrap();
    let cleared = store.clear_expired().await.unwrap();
    assert!(cleared >= 1);
    assert!(store.load(&expired_key).await.unwrap().is_none());
}
