use super::*;
use hyper::http::StatusCode;

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
