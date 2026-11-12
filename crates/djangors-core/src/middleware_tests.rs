use super::*;
use crate::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use hyper::header::{COOKIE, SET_COOKIE};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tower::service_fn;
use tower::ServiceExt;

#[test]
fn test_constant_time_eq() {
    assert!(constant_time_eq(b"", b""));
    assert!(constant_time_eq(b"hello", b"hello"));
    assert!(!constant_time_eq(b"hello", b"world"));
    assert!(!constant_time_eq(b"hello", b"hell"));
    assert!(!constant_time_eq(b"hell", b"hello"));
}

#[tokio::test]
async fn test_safe_method_no_cookie_no_header() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = CsrfLayer::new().layer(inner_svc);

    let req = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Response must set a fresh csrftoken cookie
    let set_cookie_header = resp.headers().get(SET_COOKIE).expect("Should set cookie");
    let set_cookie_str = set_cookie_header.to_str().unwrap();
    assert!(set_cookie_str.contains("csrftoken="));
    assert!(set_cookie_str.contains("Path=/"));
    assert!(set_cookie_str.contains("SameSite=Lax"));
    assert!(set_cookie_str.contains("Max-Age=31536000"));
    assert!(!set_cookie_str.contains("HttpOnly"));
}

#[tokio::test]
async fn test_unsafe_method_no_cookie_no_header_rejected() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |req: hyper::Request<Full<Bytes>>| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let pending = req
            .extensions()
            .get::<CsrfPendingFormCheck>()
            .expect("CsrfPendingFormCheck should exist");
        let token = req
            .extensions()
            .get::<CsrfToken>()
            .expect("CsrfToken should exist");
        assert_eq!(pending.0, token.0);

        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = CsrfLayer::new().layer(inner_svc);

    let req = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_unsafe_method_cookie_present_missing_or_mismatched_header_rejected() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |req: hyper::Request<Full<Bytes>>| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let pending = req
            .extensions()
            .get::<CsrfPendingFormCheck>()
            .expect("CsrfPendingFormCheck should exist");
        assert_eq!(pending.0, "validtoken1234567890123456789012");
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = CsrfLayer::new().layer(inner_svc);

    // Case 1: Cookie present, header missing
    let req1 = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .header(COOKIE, "csrftoken=validtoken1234567890123456789012")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
    assert_eq!(resp1.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Case 2: Cookie present, header mismatched
    let counter2 = Arc::new(AtomicUsize::new(0));
    let counter2_clone = counter2.clone();
    let inner_svc2 = service_fn(move |req: hyper::Request<Full<Bytes>>| {
        counter2_clone.fetch_add(1, Ordering::SeqCst);
        let pending = req
            .extensions()
            .get::<CsrfPendingFormCheck>()
            .expect("CsrfPendingFormCheck should exist");
        assert_eq!(pending.0, "validtoken1234567890123456789012");
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });
    let mut svc2 = CsrfLayer::new().layer(inner_svc2);

    let req2 = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .header(COOKIE, "csrftoken=validtoken1234567890123456789012")
        .header("X-CSRFToken", "differentvalidtokenvaluehere12345")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp2 = svc2.ready().await.unwrap().call(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(counter2.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_unsafe_method_cookie_present_matching_header_succeeds() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = CsrfLayer::new().layer(inner_svc);

    let req = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .header(COOKIE, "csrftoken=validtoken123")
        .header("X-CSRFToken", "validtoken123")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_end_to_end_router_csrf() {
    use tower::service_fn;
    use tower::ServiceBuilder;

    async fn test_handler(req: Request, _: PathParams) -> Result<Response, DjangorsError> {
        let token = req.ext::<CsrfToken>().expect("CsrfToken should exist");
        Ok(Response::text(StatusCode::OK, &token.0))
    }

    let router = Router::new().get("/", test_handler).post("/", test_handler);

    let layer = CsrfLayer::new();

    let svc_fn = service_fn(move |req: hyper::Request<Full<Bytes>>| {
        let router = router.clone();
        async move { Ok::<_, Infallible>(router.dispatch(req).await) }
    });

    let mut svc = ServiceBuilder::new().layer(layer).service(svc_fn);

    // 1. GET to obtain Set-Cookie
    let req1 = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);

    let set_cookie_header = resp1.headers().get(SET_COOKIE).expect("Should set cookie");
    let set_cookie_str = set_cookie_header.to_str().unwrap().to_string();
    let cookie_value = set_cookie_str
        .split(';')
        .next()
        .unwrap()
        .split_once('=')
        .unwrap()
        .1
        .to_string();

    // 2. POST carrying cookie and matching header
    let req2 = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .header(COOKIE, format!("csrftoken={}", cookie_value))
        .header("X-CSRFToken", &cookie_value)
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);

    // 3. POST carrying cookie but wrong/mismatched header value
    let req3 = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .header(COOKIE, format!("csrftoken={}", cookie_value))
        .header("X-CSRFToken", "mismatchedtokenvalue1234567890123")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp3 = svc.ready().await.unwrap().call(req3).await.unwrap();
    assert_eq!(resp3.status(), StatusCode::FORBIDDEN);

    // 4. POST carrying cookie, no X-CSRFToken header, but valid csrfmiddlewaretoken in body
    let body_content = format!("csrfmiddlewaretoken={}", cookie_value);
    let req4 = hyper::Request::builder()
        .method("POST")
        .uri("/")
        .header(COOKIE, format!("csrftoken={}", cookie_value))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(Full::new(Bytes::from(body_content)))
        .unwrap();

    let resp4 = svc.ready().await.unwrap().call(req4).await.unwrap();
    assert_eq!(resp4.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_host_validation_success() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |_req| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc =
        HostValidationLayer::new(vec!["example.com".to_string(), "localhost".to_string()])
            .layer(inner_svc);

    // Case 1: allowed host
    let req1 = hyper::Request::builder()
        .header("Host", "example.com")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp1 = svc.ready().await.unwrap().call(req1).await.unwrap();
    assert_eq!(resp1.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // Case 2: allowed host with port
    let req2 = hyper::Request::builder()
        .header("Host", "localhost:8080")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp2 = svc.ready().await.unwrap().call(req2).await.unwrap();
    assert_eq!(resp2.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_host_validation_failure() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |_req| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = HostValidationLayer::new(vec!["example.com".to_string()]).layer(inner_svc);

    let req = hyper::Request::builder()
        .header("Host", "attacker.com")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(counter.load(Ordering::SeqCst), 0); // never called
}

#[tokio::test]
async fn test_host_validation_empty_unrestricted() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let inner_svc = service_fn(move |_req| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = HostValidationLayer::new(vec![]).layer(inner_svc);

    let req = hyper::Request::builder()
        .header("Host", "any-random-host.com")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_hsts_layer() {
    let inner_svc = service_fn(move |_req| {
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    // 1. Without subdomains
    let mut svc_no_sub = HstsLayer::new(31536000).layer(inner_svc);
    let req1 = hyper::Request::builder()
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp1 = svc_no_sub.ready().await.unwrap().call(req1).await.unwrap();
    let hsts_hdr1 = resp1.headers().get("strict-transport-security").unwrap();
    assert_eq!(hsts_hdr1.to_str().unwrap(), "max-age=31536000");

    // 2. With subdomains
    let mut svc_sub = HstsLayer::new(31536000)
        .with_include_subdomains(true)
        .layer(inner_svc);
    let req2 = hyper::Request::builder()
        .method("POST")
        .uri("/some-path")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp2 = svc_sub.ready().await.unwrap().call(req2).await.unwrap();
    let hsts_hdr2 = resp2.headers().get("strict-transport-security").unwrap();
    assert_eq!(
        hsts_hdr2.to_str().unwrap(),
        "max-age=31536000; includeSubDomains"
    );
}

#[test]
fn test_csp_builder_assembles_directives_in_call_order() {
    let csp = CspBuilder::new()
        .default_src(&["'self'"])
        .script_src(&["'self'", "https://cdn.example.com"])
        .img_src(&["'self'", "data:"])
        .build();
    assert_eq!(csp.header_name, "content-security-policy");
    assert_eq!(
        csp.header_value,
        "default-src 'self'; script-src 'self' https://cdn.example.com; img-src 'self' data:"
    );
}

#[test]
fn test_csp_builder_upgrade_insecure_requests_is_a_bare_directive() {
    let csp = CspBuilder::new()
        .default_src(&["'self'"])
        .upgrade_insecure_requests()
        .build();
    assert_eq!(
        csp.header_value,
        "default-src 'self'; upgrade-insecure-requests"
    );
}

#[test]
fn test_csp_builder_report_only_uses_the_report_only_header_name() {
    let csp = CspBuilder::new()
        .default_src(&["'self'"])
        .report_only(true)
        .build();
    assert_eq!(csp.header_name, "content-security-policy-report-only");
}

#[tokio::test]
async fn test_csp_layer_sets_the_real_response_header() {
    let inner_svc = service_fn(move |_req| {
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = CspBuilder::new()
        .default_src(&["'self'"])
        .frame_ancestors(&["'none'"])
        .build()
        .layer(inner_svc);

    let req = hyper::Request::builder()
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp = svc.ready().await.unwrap().call(req).await.unwrap();
    let csp_hdr = resp.headers().get("content-security-policy").unwrap();
    assert_eq!(
        csp_hdr.to_str().unwrap(),
        "default-src 'self'; frame-ancestors 'none'"
    );
}

#[tokio::test]
async fn test_csrf_secure_flag() {
    let inner_svc = service_fn(move |_req| {
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    // 1. Default (secure: false)
    let mut svc_default = CsrfLayer::new().layer(inner_svc);
    let req1 = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp1 = svc_default.ready().await.unwrap().call(req1).await.unwrap();
    let set_cookie1 = resp1.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(!set_cookie1.contains("; Secure"));

    // 2. secure: true
    let mut svc_secure = CsrfLayer::new().with_secure(true).layer(inner_svc);
    let req2 = hyper::Request::builder()
        .method("GET")
        .uri("/")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp2 = svc_secure.ready().await.unwrap().call(req2).await.unwrap();
    let set_cookie2 = resp2.headers().get(SET_COOKIE).unwrap().to_str().unwrap();
    assert!(set_cookie2.contains("; Secure"));
}

#[tokio::test]
async fn test_cors_preflight_allowed_and_disallowed_origins() {
    let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let config = CorsConfig::new()
        .allowed_origins(["https://app.example.com"])
        .allow_credentials(true);
    let layer = cors_layer(&config).unwrap();
    let mut svc = layer.layer(inner_svc);

    // Allowed origin preflight
    let req_allowed = hyper::Request::builder()
        .method("OPTIONS")
        .uri("/api/items")
        .header("origin", "https://app.example.com")
        .header("access-control-request-method", "POST")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp_allowed = svc.ready().await.unwrap().call(req_allowed).await.unwrap();
    assert_eq!(resp_allowed.status(), 200);
    assert_eq!(
        resp_allowed
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "https://app.example.com"
    );
    assert!(resp_allowed
        .headers()
        .get("access-control-allow-methods")
        .is_some());
    assert_eq!(
        resp_allowed
            .headers()
            .get("access-control-allow-credentials")
            .unwrap(),
        "true"
    );

    // Disallowed origin preflight gets no CORS headers
    let req_disallowed = hyper::Request::builder()
        .method("OPTIONS")
        .uri("/api/items")
        .header("origin", "https://evil.example.com")
        .header("access-control-request-method", "POST")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp_disallowed = svc
        .ready()
        .await
        .unwrap()
        .call(req_disallowed)
        .await
        .unwrap();
    assert!(resp_disallowed
        .headers()
        .get("access-control-allow-origin")
        .is_none());
}

#[test]
fn test_cors_wildcard_with_credentials_rejected() {
    let config = CorsConfig::new()
        .allowed_origins(["*"])
        .allow_credentials(true);
    assert!(cors_layer(&config).is_err());

    let invalid_origin = CorsConfig::new().allowed_origins(["not a valid origin"]);
    assert!(cors_layer(&invalid_origin).is_err());
}

#[test]
fn test_cors_origin_must_not_have_a_path() {
    for origin in [
        "https://app.example.com/",
        "https://app.example.com/path",
        "https://app.example.com?x=1",
        "app.example.com",
    ] {
        let config = CorsConfig::new().allowed_origins([origin]);
        assert!(cors_layer(&config).is_err(), "{origin} should be rejected");
    }
    let ok = CorsConfig::new().allowed_origins(["https://app.example.com:8443"]);
    assert!(cors_layer(&ok).is_ok());
}

#[test]
fn test_cors_config_from_settings_uses_configured_origins() {
    let settings = crate::settings::DjangorsSettings::default()
        .with_cors_allowed_origins(["https://app.example.com"]);
    let config = CorsConfig::from_settings(&settings);
    assert_eq!(
        config.allowed_origins,
        vec!["https://app.example.com".to_string()]
    );
}
