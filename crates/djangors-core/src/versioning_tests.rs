use super::*;
use crate::error::ErrorRenderer;
use crate::state::AppState;
use crate::ProblemDetailsRenderer;
use hyper::http::{Method, Uri};
use tower::service_fn;
use tower::ServiceExt;

fn make_request(uri: &str, headers: HeaderMap, versioning: Versioning) -> Request {
    Request::new(
        Method::GET,
        uri.parse::<Uri>().unwrap(),
        headers,
        Bytes::new(),
    )
    .with_state(AppState::new().insert(versioning))
}

#[tokio::test]
async fn extracts_from_url_prefix_header_and_media_type() {
    let url_ver = Versioning::new(VersionSource::UrlPrefix).supported(["1", "2"]);
    let req_url = make_request("/v2/books", HeaderMap::new(), url_ver);
    let ApiVersion(v) = ApiVersion::from_request(&req_url).await.unwrap();
    assert_eq!(v, "2");

    let header_ver = Versioning::new(VersionSource::Header("X-Api-Version")).supported(["1", "2"]);
    let mut headers = HeaderMap::new();
    headers.insert("x-api-version", HeaderValue::from_static("1"));
    let req_header = make_request("/books", headers, header_ver);
    let ApiVersion(v) = ApiVersion::from_request(&req_header).await.unwrap();
    assert_eq!(v, "1");

    let media_ver = Versioning::new(VersionSource::MediaType("version")).supported(["1", "2"]);
    let mut media_headers = HeaderMap::new();
    media_headers.insert(
        hyper::header::ACCEPT,
        HeaderValue::from_static("application/json; version=2"),
    );
    let req_media = make_request("/books", media_headers, media_ver);
    let ApiVersion(v) = ApiVersion::from_request(&req_media).await.unwrap();
    assert_eq!(v, "2");
}

#[tokio::test]
async fn falls_back_to_default_version() {
    let versioning = Versioning::new(VersionSource::Header("X-Api-Version"))
        .default_version("1")
        .supported(["1", "2"]);
    let req = make_request("/books", HeaderMap::new(), versioning);
    let ApiVersion(v) = ApiVersion::from_request(&req).await.unwrap();
    assert_eq!(v, "1");
}

#[tokio::test]
async fn unsupported_or_missing_version_returns_400_problem_details() {
    let versioning = Versioning::new(VersionSource::Header("X-Api-Version")).supported(["1", "2"]);
    let mut headers = HeaderMap::new();
    headers.insert("x-api-version", HeaderValue::from_static("99"));
    let req_unsupported = make_request("/books", headers, versioning.clone());
    let err = ApiVersion::from_request(&req_unsupported)
        .await
        .unwrap_err();
    assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(err.code(), "unsupported_version");

    let problem_resp = ProblemDetailsRenderer::new().render(&err, &req_unsupported);
    assert_eq!(problem_resp.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = serde_json::from_slice(problem_resp.body()).unwrap();
    assert_eq!(body["code"], "unsupported_version");
    assert_eq!(body["status"], 400);

    let req_missing = make_request("/books", HeaderMap::new(), versioning);
    let missing_err = ApiVersion::from_request(&req_missing).await.unwrap_err();
    assert_eq!(missing_err.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(missing_err.code(), "unsupported_version");
}

#[tokio::test]
async fn deprecation_layer_sets_headers_only_on_deprecated_versions() {
    let versioning = Versioning::new(VersionSource::Header("X-Api-Version"))
        .supported(["1", "2"])
        .deprecate("1", Some("Wed, 11 Nov 2026 11:11:11 GMT"));

    let inner_svc = service_fn(move |_req: hyper::Request<Full<Bytes>>| {
        let resp = hyper::Response::builder()
            .status(200)
            .body(Full::new(Bytes::from("ok")))
            .unwrap();
        async move { Ok::<_, Infallible>(resp) }
    });

    let mut svc = DeprecationLayer::new(versioning).layer(inner_svc);

    let req_dep = hyper::Request::builder()
        .method("GET")
        .uri("/books")
        .header("x-api-version", "1")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp_dep = svc.ready().await.unwrap().call(req_dep).await.unwrap();
    assert_eq!(resp_dep.headers().get("deprecation").unwrap(), "true");
    assert_eq!(
        resp_dep.headers().get("sunset").unwrap(),
        "Wed, 11 Nov 2026 11:11:11 GMT"
    );

    let req_cur = hyper::Request::builder()
        .method("GET")
        .uri("/books")
        .header("x-api-version", "2")
        .body(Full::new(Bytes::new()))
        .unwrap();
    let resp_cur = svc.ready().await.unwrap().call(req_cur).await.unwrap();
    assert!(resp_cur.headers().get("deprecation").is_none());
    assert!(resp_cur.headers().get("sunset").is_none());
}

#[tokio::test]
async fn url_prefix_without_a_version_segment_uses_the_default() {
    let versioning = Versioning::new(VersionSource::UrlPrefix)
        .default_version("1")
        .supported(["1", "2"]);
    let req = make_request("/books", HeaderMap::new(), versioning.clone());
    assert_eq!(
        ApiVersion::from_request(&req).await.unwrap(),
        ApiVersion("1".into())
    );

    let req = make_request("/v2/books", HeaderMap::new(), versioning.clone());
    assert_eq!(
        ApiVersion::from_request(&req).await.unwrap(),
        ApiVersion("2".into())
    );

    // `/videos` starts with `v` but is not a version.
    let req = make_request("/videos", HeaderMap::new(), versioning);
    assert_eq!(
        ApiVersion::from_request(&req).await.unwrap(),
        ApiVersion("1".into())
    );
}
