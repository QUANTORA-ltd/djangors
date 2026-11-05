use super::*;
use bytes::Bytes;
use hyper::http::{HeaderMap, Method, StatusCode, Uri};
use std::str::FromStr;

fn make_request(method: Method, path: &str) -> Request {
    let uri = Uri::from_str(path).expect("valid URI");
    Request::new(method, uri, HeaderMap::new(), Bytes::new())
}

async fn ok_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "ok"))
}

async fn echo_name_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let name = params.get("name").unwrap_or("?");
    Ok(Response::text(StatusCode::OK, name))
}

async fn echo_id_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let id: i64 = params.get_as("id").unwrap_or(0);
    Ok(Response::text(StatusCode::OK, &format!("item {id}")))
}

async fn echo_slug_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let slug = params.get("slug").unwrap_or("?");
    Ok(Response::text(StatusCode::OK, slug))
}

async fn echo_two_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let a = params.get("a").unwrap_or("?");
    let b: i64 = params.get_as("b").unwrap_or(0);
    Ok(Response::text(StatusCode::OK, &format!("{a}/{b}")))
}

async fn get_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "get"))
}

async fn post_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "post"))
}

async fn mount_handler_fn(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let id = params.get("id").unwrap_or("?");
    Ok(Response::text(StatusCode::OK, id))
}

async fn root_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "root"))
}

async fn panicking_handler_fn(_: Request, _: PathParams) -> Result<Response, DjangorsError> {
    panic!("boom");
}

async fn matched_route_fn(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
    let route = req.matched_route().unwrap_or("none").to_string();
    Ok(Response::text(StatusCode::OK, &route))
}

#[tokio::test]
async fn matched_route_is_available_to_handler_and_response() {
    let router = Router::new().get("/items/{id:i64}", matched_route_fn);
    let req = make_request(Method::GET, "/items/42");
    let resp = router.handle(req).await.unwrap();

    // The handler saw the pattern, not the raw path.
    assert_eq!(resp.body(), b"/items/{id:i64}");

    // An outer tower layer can read it from the response extensions.
    let matched = resp
        .extensions()
        .get::<MatchedRoute>()
        .expect("router inserts MatchedRoute into the response");
    assert_eq!(matched.as_str(), "/items/{id:i64}");
}

#[tokio::test]
async fn matched_route_is_absent_on_404() {
    let router = Router::new().get("/hello", ok_handler_fn);
    let req = make_request(Method::GET, "/nope");
    assert!(router.handle(req).await.is_err());
}

#[tokio::test]
async fn literal_route_match() {
    let router = Router::new().get("/hello", ok_handler_fn);
    let req = make_request(Method::GET, "/hello");
    let resp = router.handle(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn literal_route_no_match() {
    let router = Router::new().get("/hello", ok_handler_fn);
    let req = make_request(Method::GET, "/world");
    let result = router.handle(req).await;
    assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
}

#[tokio::test]
async fn method_mismatch() {
    let router = Router::new().get("/hello", ok_handler_fn);
    let req = make_request(Method::POST, "/hello");
    let result = router.handle(req).await;
    assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
}

#[tokio::test]
async fn string_capture() {
    let router = Router::new().get("/hello/{name}", echo_name_fn);
    let req = make_request(Method::GET, "/hello/world");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "world");
}

#[tokio::test]
async fn colon_alias_capture() {
    // `:name` must behave exactly like the untyped `{name}` capture, not
    // like a literal segment that can never match a real request path.
    let router = Router::new().get("/hello/:name", echo_name_fn);
    let req = make_request(Method::GET, "/hello/world");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "world");
}

#[tokio::test]
async fn i64_capture_valid() {
    let router = Router::new().get("/item/{id:i64}", echo_id_fn);
    let req = make_request(Method::GET, "/item/42");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "item 42");
}

#[tokio::test]
async fn i64_capture_invalid() {
    let router = Router::new().get("/item/{id:i64}", echo_id_fn);
    let req = make_request(Method::GET, "/item/abc");
    let result = router.handle(req).await;
    assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
}

#[tokio::test]
async fn slug_capture_valid() {
    let router = Router::new().get("/post/{slug:slug}", echo_slug_fn);
    let req = make_request(Method::GET, "/post/my-blog_post-123");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "my-blog_post-123");
}

#[tokio::test]
async fn slug_capture_invalid() {
    let router = Router::new().get("/post/{slug:slug}", echo_slug_fn);
    let req = make_request(Method::GET, "/post/hello.world");
    let result = router.handle(req).await;
    assert!(matches!(result.unwrap_err(), DjangorsError::NotFound));
}

#[tokio::test]
async fn multiple_captures() {
    let router = Router::new().get("/{a}/{b:i64}", echo_two_fn);
    let req = make_request(Method::GET, "/foo/99");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "foo/99");
}

#[tokio::test]
async fn post_route() {
    let router = Router::new()
        .get("/a", get_handler_fn)
        .post("/a", post_handler_fn);
    let req_post = make_request(Method::POST, "/a");
    let resp = router.handle(req_post).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "post");
}

#[tokio::test]
async fn mount_sub_router() {
    let sub = Router::new().get("/users/{id}", mount_handler_fn);
    let router = Router::new().mount("/api", sub);
    let req = make_request(Method::GET, "/api/users/7");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "7");
}

#[tokio::test]
async fn root_path_match() {
    let router = Router::new().get("/", root_handler_fn);
    let req = make_request(Method::GET, "/");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "root");
}

#[tokio::test]
async fn integration_dispatch() {
    async fn handler_func(_: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let name = params.get("name").unwrap_or("?");
        Ok(Response::text(StatusCode::OK, &format!("Hello, {name}!")))
    }

    let router = Router::new().get("/greet/{name}", handler_func);

    let req = make_request(Method::GET, "/greet/Alice");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "Hello, Alice!");
}

#[tokio::test]
async fn plain_async_fn_handler_no_wrapping() {
    async fn greet(_req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let name = params.get("name").unwrap_or("stranger");
        Ok(Response::text(StatusCode::OK, &format!("Hello, {name}!")))
    }

    let router = Router::new().get("/greet/{name}", greet);

    let req = make_request(Method::GET, "/greet/World");
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "Hello, World!");
}

#[tokio::test]
async fn test_handler_panic_recovery() {
    let router = Router::new().get("/panic", panicking_handler_fn);
    let req = make_request(Method::GET, "/panic");
    let res = router.handle(req).await;

    assert!(res.is_err());
    let err = res.unwrap_err();
    if let DjangorsError::Panicked(msg) = err {
        assert_eq!(msg, "boom");
    } else {
        panic!("Expected DjangorsError::Panicked, got {:?}", err);
    }
}

#[tokio::test]
async fn test_dispatch_debug_panic() {
    let router = Router::new().get("/panic", panicking_handler_fn);

    // Dispatch with debug = true
    let hyper_req = hyper::Request::builder()
        .method("GET")
        .uri("/panic")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp = router.dispatch_debug(hyper_req, true).await;
    assert_eq!(hyper_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    use http_body_util::BodyExt;
    let body_bytes = hyper_resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("Handler Panicked"));
    assert!(body_str.contains("boom"));
    assert!(body_str.contains("DEBUG = true"));

    // Dispatch with debug = false
    let hyper_req_prod = hyper::Request::builder()
        .method("GET")
        .uri("/panic")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp_prod = router.dispatch_debug(hyper_req_prod, false).await;
    assert_eq!(hyper_resp_prod.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body_bytes_prod = hyper_resp_prod
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let body_str_prod = String::from_utf8(body_bytes_prod.to_vec()).unwrap();
    assert!(!body_str_prod.contains("boom"));
    assert!(!body_str_prod.contains("DEBUG = true"));
    assert!(body_str_prod.contains("Internal Server Error"));
}

#[tokio::test]
async fn test_router_signals_fire() {
    use crate::signals::{REQUEST_FINISHED, REQUEST_STARTED};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let started_counter = Arc::new(AtomicUsize::new(0));
    let finished_counter = Arc::new(AtomicUsize::new(0));

    let started_clone = started_counter.clone();
    REQUEST_STARTED.connect(move |payload| {
        let started = started_clone.clone();
        async move {
            if payload.path == "/signals-test-unique-path-foo" {
                started.fetch_add(1, Ordering::SeqCst);
                assert_eq!(payload.method, "GET");
            }
        }
    });

    let finished_clone = finished_counter.clone();
    REQUEST_FINISHED.connect(move |payload| {
        let finished = finished_clone.clone();
        async move {
            if payload.path == "/signals-test-unique-path-foo" {
                finished.fetch_add(1, Ordering::SeqCst);
                assert_eq!(payload.method, "GET");
                assert_eq!(payload.status, 200);
            }
        }
    });

    let router = Router::new().get(
        "/signals-test-unique-path-foo",
        |_: Request, _: PathParams| async { Ok(Response::text(StatusCode::OK, "ok")) },
    );

    let req = make_request(Method::GET, "/signals-test-unique-path-foo");
    let _resp = router.handle(req).await.unwrap();

    assert_eq!(started_counter.load(Ordering::SeqCst), 1);
    assert_eq!(finished_counter.load(Ordering::SeqCst), 1);
}

#[derive(Clone)]
struct DbConnectionPool(String);

async fn state_handler_fn(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
    let pool = req.state::<DbConnectionPool>().expect("pool should exist");
    Ok(Response::text(StatusCode::OK, &pool.0))
}

#[tokio::test]
async fn test_router_app_state() {
    let router = Router::new()
        .with_state(DbConnectionPool("postgres://localhost".to_string()))
        .get("/state-test", state_handler_fn);

    let req = make_request(Method::GET, "/state-test").with_state(router.state.clone());
    let resp = router.handle(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_str = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body_str, "postgres://localhost");
}

#[derive(Clone)]
struct FakeSession(String);

async fn ext_handler_fn(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
    let session = req.ext::<FakeSession>().expect("extension should exist");
    Ok(Response::text(StatusCode::OK, &session.0))
}

#[tokio::test]
async fn test_dispatch_propagates_hyper_extensions_to_handler() {
    let router = Router::new().get("/ext-test", ext_handler_fn);

    let mut hyper_req = hyper::Request::builder()
        .method(Method::GET)
        .uri("/ext-test")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    hyper_req
        .extensions_mut()
        .insert(FakeSession("injected-by-middleware".to_string()));

    let hyper_resp = router.dispatch(hyper_req).await;
    assert_eq!(hyper_resp.status(), StatusCode::OK);

    use http_body_util::BodyExt;
    let body_bytes = hyper_resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_bytes[..], b"injected-by-middleware");
}

#[tokio::test]
async fn test_default_error_rendering_is_unchanged_without_a_registered_renderer() {
    // No ErrorRenderer registered in state: dispatch/dispatch_debug/dispatch_boxed must all
    // fall back to their pre-existing default behavior exactly as before this feature landed.
    let router = Router::new();

    let hyper_req = hyper::Request::builder()
        .method("GET")
        .uri("/does-not-exist")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp = router.dispatch(hyper_req).await;
    assert_eq!(hyper_resp.status(), StatusCode::NOT_FOUND);
    use http_body_util::BodyExt;
    let body = hyper_resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"404 Not Found");

    let hyper_req_debug = hyper::Request::builder()
        .method("GET")
        .uri("/does-not-exist")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp_debug = router.dispatch_debug(hyper_req_debug, false).await;
    assert_eq!(hyper_resp_debug.status(), StatusCode::NOT_FOUND);
    let body_debug = hyper_resp_debug
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let body_debug_str = String::from_utf8(body_debug.to_vec()).unwrap();
    assert!(body_debug_str.contains("<!DOCTYPE html>"));

    let hyper_req_boxed = hyper::Request::builder()
        .method("GET")
        .uri("/does-not-exist")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp_boxed = router.dispatch_boxed(hyper_req_boxed, false).await;
    assert_eq!(hyper_resp_boxed.status(), StatusCode::NOT_FOUND);
    let body_boxed = hyper_resp_boxed
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let body_boxed_str = String::from_utf8(body_boxed.to_vec()).unwrap();
    assert!(body_boxed_str.contains("<!DOCTYPE html>"));
}

#[tokio::test]
async fn test_registered_error_renderer_overrides_dispatch() {
    let router = Router::new().with_state(std::sync::Arc::new(crate::error::JsonErrorRenderer)
        as std::sync::Arc<dyn crate::error::ErrorRenderer>);

    let hyper_req = hyper::Request::builder()
        .method("GET")
        .uri("/does-not-exist")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp = router.dispatch(hyper_req).await;
    assert_eq!(hyper_resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        hyper_resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json; charset=utf-8")
    );
    use http_body_util::BodyExt;
    let body = hyper_resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["status"], 404);
    assert_eq!(json["error"]["code"], "not_found");
}

#[tokio::test]
async fn test_set_error_renderer_matches_manual_cast() {
    let router_manual = Router::new()
        .with_state(std::sync::Arc::new(crate::error::JsonErrorRenderer)
            as std::sync::Arc<dyn crate::error::ErrorRenderer>);
    let router_helper = Router::new().set_error_renderer(crate::error::JsonErrorRenderer);

    let hyper_req_manual = hyper::Request::builder()
        .method("GET")
        .uri("/does-not-exist")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp_manual = router_manual.dispatch(hyper_req_manual).await;

    let hyper_req_helper = hyper::Request::builder()
        .method("GET")
        .uri("/does-not-exist")
        .body(http_body_util::Full::new(Bytes::new()))
        .unwrap();
    let hyper_resp_helper = router_helper.dispatch(hyper_req_helper).await;

    assert_eq!(hyper_resp_manual.status(), hyper_resp_helper.status());
    assert_eq!(
        hyper_resp_manual.headers().get("content-type"),
        hyper_resp_helper.headers().get("content-type")
    );

    use http_body_util::BodyExt;
    let body_manual = hyper_resp_manual
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    let body_helper = hyper_resp_helper
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(body_manual, body_helper);
}

#[tokio::test]
async fn test_registered_error_renderer_overrides_dispatch_debug_in_both_modes() {
    let router = Router::new()
        .get("/panic", panicking_handler_fn)
        .with_state(std::sync::Arc::new(crate::error::JsonErrorRenderer)
            as std::sync::Arc<dyn crate::error::ErrorRenderer>);

    for debug in [true, false] {
        let hyper_req = hyper::Request::builder()
            .method("GET")
            .uri("/panic")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let hyper_resp = router.dispatch_debug(hyper_req, debug).await;
        assert_eq!(hyper_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        use http_body_util::BodyExt;
        let body = hyper_resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "panicked");
        assert_eq!(json["error"]["message"], "boom");
    }
}

#[tokio::test]
async fn test_registered_error_renderer_overrides_dispatch_boxed_in_both_modes() {
    let router = Router::new()
        .get("/panic", panicking_handler_fn)
        .with_state(std::sync::Arc::new(crate::error::JsonErrorRenderer)
            as std::sync::Arc<dyn crate::error::ErrorRenderer>);

    for debug in [true, false] {
        let hyper_req = hyper::Request::builder()
            .method("GET")
            .uri("/panic")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();
        let hyper_resp = router.dispatch_boxed(hyper_req, debug).await;
        assert_eq!(hyper_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        use http_body_util::BodyExt;
        let body = hyper_resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["code"], "panicked");
        assert_eq!(json["error"]["message"], "boom");
    }
}

#[tokio::test]
async fn test_reverse_round_trip() {
    let router = Router::new()
        .get("/polls/{id}", echo_id_fn)
        .name("poll-detail");

    let reversed = router.reverse("poll-detail", &[("id", "42")]).unwrap();
    assert_eq!(reversed, "/polls/42");

    let req = make_request(Method::GET, &reversed);
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "item 42");
}

#[tokio::test]
async fn test_reverse_mounted_sub_router() {
    let sub = Router::new()
        .get("/users/{id}", echo_id_fn)
        .name("user-detail");
    let router = Router::new().mount("/api/v1", sub);

    let reversed = router.reverse("user-detail", &[("id", "99")]).unwrap();
    assert_eq!(reversed, "/api/v1/users/99");

    let req = make_request(Method::GET, &reversed);
    let resp = router.handle(req).await.unwrap();
    let body = String::from_utf8(resp.body().to_vec()).unwrap();
    assert_eq!(body, "item 99");
}

#[test]
fn test_reverse_errors() {
    let router = Router::new()
        .get("/polls/{id}", ok_handler_fn)
        .name("poll-detail");

    // Unknown name
    let err = router.reverse("unknown", &[]).unwrap_err();
    assert_eq!(err, ReverseError::UnknownName("unknown".to_string()));

    // Missing param
    let err = router.reverse("poll-detail", &[]).unwrap_err();
    assert_eq!(
        err,
        ReverseError::MissingParam {
            route: "poll-detail".to_string(),
            param: "id".to_string(),
        }
    );

    // Unexpected param
    let err = router
        .reverse("poll-detail", &[("id", "1"), ("extra", "2")])
        .unwrap_err();
    assert_eq!(
        err,
        ReverseError::UnexpectedParam {
            route: "poll-detail".to_string(),
            param: "extra".to_string(),
        }
    );
}

#[test]
fn test_reverse_percent_encoding() {
    let router = Router::new()
        .get("/search/{query}", ok_handler_fn)
        .name("search");

    let reversed = router
        .reverse("search", &[("query", "hello / world? #1")])
        .unwrap();
    assert_eq!(reversed, "/search/hello%20%2F%20world%3F%20%231");
}

#[test]
#[should_panic(expected = "Router::name called on an empty router")]
fn test_name_on_empty_router_panics() {
    Router::new().name("empty");
}

#[test]
#[should_panic(expected = "Duplicate route name 'poll-detail'")]
fn test_duplicate_route_name_panics() {
    Router::new()
        .get("/polls/{id}", ok_handler_fn)
        .name("poll-detail")
        .get("/polls/view/{id}", ok_handler_fn)
        .name("poll-detail");
}

#[test]
#[should_panic(expected = "Duplicate route name 'poll-detail'")]
fn test_duplicate_route_name_mount_panics() {
    let sub = Router::new()
        .get("/polls/{id}", ok_handler_fn)
        .name("poll-detail");
    Router::new()
        .get("/main/polls/{id}", ok_handler_fn)
        .name("poll-detail")
        .mount("/sub", sub);
}
