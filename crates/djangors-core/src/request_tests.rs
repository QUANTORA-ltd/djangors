use super::*;
use hyper::http::Uri;

#[tokio::test]
async fn body_bytes() {
    let body = Bytes::from("hello");
    let req = Request::new(Method::GET, Uri::from_static("/"), HeaderMap::new(), body);
    assert_eq!(req.body_bytes().await, b"hello");
}

#[tokio::test]
async fn method_and_path() {
    let req = Request::new(
        Method::POST,
        Uri::from_static("/foo/bar"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(req.method(), &Method::POST);
    assert_eq!(req.path(), "/foo/bar");
}

#[tokio::test]
async fn header_access() {
    let mut headers = HeaderMap::new();
    headers.insert("x-custom", HeaderValue::from_static("val"));
    let req = Request::new(Method::GET, Uri::from_static("/"), headers, Bytes::new());
    assert_eq!(req.header("x-custom").unwrap(), "val");
    assert!(req.header("missing").is_none());
}

#[test]
fn is_htmx_checks_request_header() {
    let mut h = HeaderMap::new();
    h.insert("hx-request", HeaderValue::from_static("true"));
    assert!(Request::new(Method::GET, Uri::from_static("/"), h, Bytes::new()).is_htmx());
}
#[test]
fn is_htmx_boosted_checks_header() {
    let mut h = HeaderMap::new();
    h.insert("hx-boosted", HeaderValue::from_static("true"));
    assert!(Request::new(Method::GET, Uri::from_static("/"), h, Bytes::new()).is_htmx_boosted());
}
#[test]
fn htmx_target_reads_header() {
    let mut h = HeaderMap::new();
    h.insert("hx-target", HeaderValue::from_static("results"));
    assert_eq!(
        Request::new(Method::GET, Uri::from_static("/"), h, Bytes::new()).htmx_target(),
        Some("results")
    );
}
#[test]
fn htmx_trigger_reads_header() {
    let mut h = HeaderMap::new();
    h.insert("hx-trigger", HeaderValue::from_static("search"));
    assert_eq!(
        Request::new(Method::GET, Uri::from_static("/"), h, Bytes::new()).htmx_trigger(),
        Some("search")
    );
}
#[test]
fn htmx_current_url_reads_header() {
    let mut h = HeaderMap::new();
    h.insert(
        "hx-current-url",
        HeaderValue::from_static("https://example.test/"),
    );
    assert_eq!(
        Request::new(Method::GET, Uri::from_static("/"), h, Bytes::new()).htmx_current_url(),
        Some("https://example.test/")
    );
}

#[tokio::test]
async fn query_params() {
    let req = Request::new(
        Method::GET,
        Uri::from_static("/search?q=rust&page=2"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(req.query("q"), Some("rust"));
    assert_eq!(req.query("page"), Some("2"));
    assert_eq!(req.query("missing"), None);
}

#[tokio::test]
async fn query_params_url_encoded() {
    let req = Request::new(
        Method::GET,
        Uri::from_static("/search?name=hello%20world"),
        HeaderMap::new(),
        Bytes::new(),
    );
    assert_eq!(req.query("name"), Some("hello world"));
}
