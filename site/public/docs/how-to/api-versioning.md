# How to Version an API

Djangors supports URL-prefix, header, and media-type API versioning, along with deprecation headers and per-version OpenAPI schemas.

## Configuring `Versioning` and `ApiVersion`

Register a `Versioning` configuration in router state with `Router::with_state`, then extract `ApiVersion` in handlers:

```rust,compile
# use djangors_core::extract::FromRequest;
# use djangors_core::{ApiVersion, DjangorsError, PathParams, Request, Response, Router, StatusCode, VersionSource, Versioning};
async fn list_books(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let ApiVersion(version) = ApiVersion::from_request(&req).await?;
    Ok(Response::text(StatusCode::OK, &format!("books API v{version}")))
}

# fn example() -> Router {
let versioning = Versioning::new(VersionSource::Header("X-Api-Version"))
    .default_version("1")
    .supported(["1", "2"]);

Router::new()
    .with_state(versioning)
    .get("/books", list_books)
# }
```

The three `VersionSource` strategies are:

- `VersionSource::UrlPrefix` — reads the first path segment (e.g. `/v2/books` -> `"2"`).
- `VersionSource::Header("X-Api-Version")` — reads the named request header.
- `VersionSource::MediaType("version")` — reads the named parameter from the `Accept` header (e.g. `Accept: application/json; version=2`).

If the version is missing and `default_version` is `None`, or if the version is not in `supported`, `ApiVersion` returns `400 Bad Request` with error code `unsupported_version` (rendered as RFC 9457 `application/problem+json` when `ProblemDetailsRenderer` is configured).

## URL-prefix routing with `Router::mount`

For URL-prefix versioning, mount one sub-router per version:

```rust,compile
# use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
# async fn v1_books(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
#     Ok(Response::text(StatusCode::OK, "v1"))
# }
# async fn v2_books(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
#     Ok(Response::text(StatusCode::OK, "v2"))
# }
# fn example() -> Router {
let v1 = Router::new().get("/books", v1_books);
let v2 = Router::new().get("/books", v2_books);

Router::new()
    .mount("/v1", v1)
    .mount("/v2", v2)
# }
```

## Deprecation and Sunset headers

Mark older versions with `.deprecate(version, sunset)` and wrap your service with `DeprecationLayer`:

```rust,compile
# use djangors_core::{DeprecationLayer, Router, VersionSource, Versioning};
# use tower::ServiceBuilder;
# fn example(router: Router) {
let versioning = Versioning::new(VersionSource::Header("X-Api-Version"))
    .supported(["1", "2"])
    .deprecate("1", Some("Wed, 11 Nov 2026 11:11:11 GMT"));

let service = ServiceBuilder::new()
    .layer(DeprecationLayer::new(versioning))
    .service(router);
# let _ = service;
# }
```

Responses for deprecated versions automatically receive `Deprecation: true` and, when a sunset date is set, `Sunset: <http-date>`.

## Building one OpenAPI spec per version

Use `OpenApiBuilder::version` to generate a separate OpenAPI 3.1 specification for each API version:

```rust,compile
# use djangors_rest::OpenApiBuilder;
# fn example() {
let mut v1_spec = OpenApiBuilder::new("Books API", "1.0.0");
v1_spec.version("1.0.0");

let mut v2_spec = OpenApiBuilder::new("Books API", "2.0.0");
v2_spec.version("2.0.0");
# let _ = (v1_spec.build(), v2_spec.build());
# }
```
