use super::*;
use hyper::http::StatusCode;

#[test]
fn internal_error_is_logged_but_redacted_from_production_response() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;
    #[derive(Clone)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Capture {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            CaptureWriter(self.0.clone())
        }
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(Capture(output.clone()))
        .finish();
    let req = crate::Request::new(
        hyper::Method::GET,
        hyper::Uri::from_static("/private"),
        Default::default(),
        bytes::Bytes::new(),
    );
    let response = tracing::subscriber::with_default(subscriber, || {
        crate::DjangorsError::Internal("db password wrong".into()).render(&req, false)
    });
    let logged = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    let body = String::from_utf8(response.body().to_vec()).unwrap();
    assert!(logged.contains("db password wrong"));
    assert!(!body.contains("db password wrong"));
}

#[test]
fn text_response() {
    let resp = Response::text(StatusCode::OK, "hello");
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(&resp.body[..], b"hello");
    assert_eq!(
        resp.headers.get(CONTENT_TYPE).unwrap(),
        "text/plain; charset=utf-8"
    );
}

#[test]
fn hx_trigger_sets_header() {
    assert_eq!(
        Response::new(StatusCode::OK)
            .hx_trigger("saved")
            .headers()
            .get("hx-trigger")
            .unwrap(),
        "saved"
    );
}
#[test]
fn hx_redirect_sets_header() {
    assert_eq!(
        Response::new(StatusCode::OK)
            .hx_redirect("/home")
            .headers()
            .get("hx-redirect")
            .unwrap(),
        "/home"
    );
}
#[test]
fn hx_refresh_sets_header() {
    assert_eq!(
        Response::new(StatusCode::OK)
            .hx_refresh()
            .headers()
            .get("hx-refresh")
            .unwrap(),
        "true"
    );
}
#[test]
fn hx_push_url_sets_header() {
    assert_eq!(
        Response::new(StatusCode::OK)
            .hx_push_url("/items")
            .headers()
            .get("hx-push-url")
            .unwrap(),
        "/items"
    );
}
#[test]
fn hx_retarget_sets_header() {
    assert_eq!(
        Response::new(StatusCode::OK)
            .hx_retarget("#results")
            .headers()
            .get("hx-retarget")
            .unwrap(),
        "#results"
    );
}
#[test]
fn hx_reswap_sets_header() {
    assert_eq!(
        Response::new(StatusCode::OK)
            .hx_reswap("outerHTML")
            .headers()
            .get("hx-reswap")
            .unwrap(),
        "outerHTML"
    );
}
#[test]
fn invalid_htmx_header_is_skipped_without_panicking() {
    assert!(Response::new(StatusCode::OK)
        .hx_trigger("bad\nvalue")
        .headers()
        .get("hx-trigger")
        .is_none());
}

#[test]
fn html_response() {
    let resp = Response::html(StatusCode::CREATED, "<h1>hi</h1>");
    assert_eq!(resp.status, StatusCode::CREATED);
    assert_eq!(
        resp.headers.get(CONTENT_TYPE).unwrap(),
        "text/html; charset=utf-8"
    );
}

#[test]
fn json_response() {
    #[derive(serde::Serialize)]
    struct Data {
        key: String,
    }
    let resp = Response::json(StatusCode::OK, &Data { key: "val".into() }).unwrap();
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(
        resp.headers.get(CONTENT_TYPE).unwrap(),
        "application/json; charset=utf-8"
    );
    let body_str = String::from_utf8(resp.body.to_vec()).unwrap();
    assert!(body_str.contains("\"key\""));
    assert!(body_str.contains("\"val\""));
}

#[test]
fn bytes_response() {
    let resp = Response::bytes(StatusCode::OK, "image/png", vec![1, 2, 3]);
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(&resp.body[..], &[1, 2, 3]);
    assert_eq!(resp.headers.get(CONTENT_TYPE).unwrap(), "image/png");
}

#[test]
fn redirect_response() {
    let resp = Response::redirect("/login");
    assert_eq!(resp.status, StatusCode::FOUND);
    assert_eq!(resp.headers.get(LOCATION).unwrap(), "/login");
}

#[test]
fn builder_header() {
    let resp = Response::text(StatusCode::OK, "body").header("X-Custom", "myval");
    assert_eq!(resp.headers.get("x-custom").unwrap(), "myval");
}

#[test]
fn into_hyper_conversion() {
    let resp = Response::text(StatusCode::OK, "test").with_extension(42u32);
    let hyper_resp: hyper::Response<http_body_util::Full<Bytes>> = resp.into();
    assert_eq!(hyper_resp.status(), StatusCode::OK);
    assert_eq!(hyper_resp.extensions().get::<u32>(), Some(&42));
}

#[cfg(feature = "cache")]
#[test]
fn cacheable_marks_response_extension() {
    let resp = Response::text(StatusCode::OK, "cached").cacheable();
    let hyper_resp = resp.into_hyper();
    assert!(hyper_resp
        .extensions()
        .get::<djangors_cache::CacheableResponse>()
        .is_some());
}

#[test]
fn rfc9457_problem_details_direct_json_writer() {
    use crate::error::{DjangorsError, ErrorRenderer, ProblemDetailsRenderer};
    use crate::request::Request;
    use hyper::http::{HeaderMap, Method, Uri};

    let err = DjangorsError::api(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        "Validation failed",
    )
    .with_details(serde_json::json!({ "title": ["This field is required."] }));

    let req = Request::new(
        Method::POST,
        "/api/posts".parse::<Uri>().unwrap(),
        HeaderMap::new(),
        Bytes::new(),
    );
    let renderer = ProblemDetailsRenderer::new();
    let resp = renderer.render(&err, &req);

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        resp.headers().get(CONTENT_TYPE).unwrap(),
        "application/problem+json; charset=utf-8"
    );
    let parsed: serde_json::Value = serde_json::from_slice(resp.body()).unwrap();
    assert_eq!(parsed["type"], "urn:djangors:error:validation_error");
    assert_eq!(parsed["title"], "Unprocessable Entity");
    assert_eq!(parsed["status"], 422);
    assert_eq!(parsed["code"], "validation_error");
    assert_eq!(parsed["detail"], "Validation failed");
    assert_eq!(parsed["instance"], "/api/posts");
    assert_eq!(parsed["errors"]["title"][0], "This field is required.");
}

#[test]
fn internal_error_text_is_hidden_from_json_clients_outside_debug() {
    use crate::{DjangorsError, Request};
    use hyper::http::{HeaderMap, Method, Uri};
    let mut headers = HeaderMap::new();
    headers.insert("accept", "application/json".parse().unwrap());
    let req = Request::new(
        Method::GET,
        "/x".parse::<Uri>().unwrap(),
        headers,
        Bytes::new(),
    );
    let err = DjangorsError::Internal("relation \"secret_table\" does not exist".into());

    let prod = err.render(&req, false);
    assert_eq!(prod.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = std::str::from_utf8(prod.body()).unwrap();
    assert!(!body.contains("secret_table"), "{body}");
    assert!(body.contains("Internal Server Error"), "{body}");

    let dev = err.render(&req, true);
    assert!(std::str::from_utf8(dev.body())
        .unwrap()
        .contains("secret_table"));
}
