#![deny(missing_docs)]
//! GraphQL integration for Djangors powered by [`async_graphql`].
//!
//! Provides [`execute_graphql_handler`], [`graphiql_handler`], and
//! [`graphql_router`] to mount an [`async_graphql::Schema`] onto a
//! [`djangors_core::Router`], with [`RequestHeaders`] automatically injected
//! into each GraphQL request's context data.

pub use async_graphql;

use async_graphql::{ObjectType, Schema, SubscriptionType, Variables};
use djangors_core::{DjangorsError, Handler, PathParams, Request, Response, Router};
use hyper::{HeaderMap, Method, StatusCode};
use serde::Deserialize;

/// JSON payload for a GraphQL HTTP `POST` request.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphQLRequestPayload {
    /// The GraphQL query or mutation document string.
    pub query: String,
    /// Optional name of the operation to execute when the document contains multiple operations.
    pub operation_name: Option<String>,
    /// Optional JSON object containing GraphQL variable values.
    pub variables: Option<serde_json::Value>,
}

/// Request HTTP headers injected into [`async_graphql::Request`] context data.
///
/// Resolvers can extract this via `ctx.data::<RequestHeaders>()` (or
/// `ctx.data_opt::<RequestHeaders>()`) to inspect headers such as
/// `Authorization` or custom tenant headers.
#[derive(Debug, Clone)]
pub struct RequestHeaders(pub HeaderMap);

/// Build an [`async_graphql::Request`] from a [`djangors_core::Request`] and
/// attach [`RequestHeaders`].
async fn build_graphql_request(req: &Request) -> Result<async_graphql::Request, DjangorsError> {
    let mut gql_req = match *req.method() {
        Method::POST => {
            let body = req.body_bytes().await;
            let payload: GraphQLRequestPayload = serde_json::from_slice(body).map_err(|err| {
                DjangorsError::BadRequest(format!("invalid GraphQL JSON payload: {err}"))
            })?;
            let mut gql_req = async_graphql::Request::new(payload.query);
            if let Some(op) = payload.operation_name {
                gql_req = gql_req.operation_name(op);
            }
            if let Some(vars) = payload.variables {
                gql_req = gql_req.variables(Variables::from_json(vars));
            }
            gql_req
        }
        Method::GET => {
            let query = req.query("query").ok_or_else(|| {
                DjangorsError::BadRequest("missing required `query` parameter".to_string())
            })?;
            let mut gql_req = async_graphql::Request::new(query);
            if let Some(op) = req.query("operationName") {
                gql_req = gql_req.operation_name(op);
            }
            if let Some(vars_raw) = req.query("variables") {
                let vars_json: serde_json::Value =
                    serde_json::from_str(vars_raw).map_err(|err| {
                        DjangorsError::BadRequest(format!(
                            "invalid `variables` JSON query parameter: {err}"
                        ))
                    })?;
                gql_req = gql_req.variables(Variables::from_json(vars_json));
            }
            gql_req
        }
        ref method => {
            return Err(DjangorsError::BadRequest(format!(
                "unsupported HTTP method for GraphQL: {method}"
            )));
        }
    };

    gql_req = gql_req.data(RequestHeaders(req.headers().clone()));
    Ok(gql_req)
}

async fn execute_graphql<Q, M, S>(
    schema: &Schema<Q, M, S>,
    req: &Request,
) -> Result<Response, DjangorsError>
where
    Q: ObjectType + 'static,
    M: ObjectType + 'static,
    S: SubscriptionType + 'static,
{
    let gql_req = build_graphql_request(req).await?;
    let resp = schema.execute(gql_req).await;
    Response::json(StatusCode::OK, &resp)
}

/// Create a [`Handler`] that executes GraphQL operations against `schema`.
///
/// Supports both `POST` (JSON [`GraphQLRequestPayload`] body) and `GET`
/// (`query`, optional `operationName`, and optional `variables` query
/// parameters). Request headers are injected into the GraphQL context as
/// [`RequestHeaders`].
pub fn execute_graphql_handler<Q, M, S>(schema: Schema<Q, M, S>) -> impl Handler + 'static
where
    Q: ObjectType + 'static,
    M: ObjectType + 'static,
    S: SubscriptionType + 'static,
{
    move |req: Request, _params: PathParams| {
        let schema = schema.clone();
        async move { execute_graphql(&schema, &req).await }
    }
}

/// Create a [`Handler`] that serves the GraphiQL interactive IDE HTML page
/// configured to send requests to `endpoint`.
pub fn graphiql_handler(endpoint: &str) -> impl Handler + 'static {
    let html = async_graphql::http::GraphiQLSource::build()
        .endpoint(endpoint)
        .finish();
    move |_req: Request, _params: PathParams| {
        let html = html.clone();
        async move { Ok(Response::html(StatusCode::OK, html)) }
    }
}

/// Build a [`Router`] mounting GraphQL execution and GraphiQL at `endpoint`.
///
/// Routes registered:
/// - `POST {endpoint}`: executes GraphQL queries and mutations.
/// - `GET {endpoint}`: executes a GraphQL query when the `query` query
///   parameter is present; otherwise serves the GraphiQL HTML interface.
/// - `GET {endpoint}/playground`: serves the GraphiQL HTML interface.
pub fn graphql_router<Q, M, S>(schema: Schema<Q, M, S>, endpoint: &str) -> Router
where
    Q: ObjectType + 'static,
    M: ObjectType + 'static,
    S: SubscriptionType + 'static,
{
    let endpoint_trimmed = endpoint.trim_end_matches('/');
    let playground_path = if endpoint_trimmed.is_empty() {
        "/playground".to_string()
    } else {
        format!("{endpoint_trimmed}/playground")
    };
    let graphiql_html = async_graphql::http::GraphiQLSource::build()
        .endpoint(endpoint)
        .finish();

    let get_schema = schema.clone();
    let get_html = graphiql_html.clone();

    Router::new()
        .post(endpoint, execute_graphql_handler(schema))
        .get(endpoint, move |req: Request, _params: PathParams| {
            let schema = get_schema.clone();
            let html = get_html.clone();
            async move {
                if req.query("query").is_some() {
                    execute_graphql(&schema, &req).await
                } else {
                    Ok(Response::html(StatusCode::OK, html))
                }
            }
        })
        .get(
            &playground_path,
            move |_req: Request, _params: PathParams| {
                let html = graphiql_html.clone();
                async move { Ok(Response::html(StatusCode::OK, html)) }
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::{Context, EmptySubscription, Object};
    use bytes::Bytes;
    use http_body_util::BodyExt;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tower::ServiceExt;

    struct QueryRoot;

    #[Object]
    impl QueryRoot {
        async fn hello(&self, name: Option<String>) -> String {
            format!("Hello, {}!", name.unwrap_or_else(|| "world".to_string()))
        }

        async fn auth_header(&self, ctx: &Context<'_>) -> Option<String> {
            ctx.data_opt::<RequestHeaders>()
                .and_then(|h| h.0.get("authorization"))
                .and_then(|v| v.to_str().ok())
                .map(ToString::to_string)
        }
    }

    struct MutationRoot {
        items: Arc<Mutex<Vec<String>>>,
    }

    #[Object]
    impl MutationRoot {
        async fn add_item(&self, item: String) -> Vec<String> {
            let mut items = self.items.lock().await;
            items.push(item);
            items.clone()
        }
    }

    fn test_schema() -> Schema<QueryRoot, MutationRoot, EmptySubscription> {
        Schema::build(
            QueryRoot,
            MutationRoot {
                items: Arc::new(Mutex::new(Vec::new())),
            },
            EmptySubscription,
        )
        .finish()
    }

    async fn read_body_string<B>(resp: hyper::Response<B>) -> String
    where
        B: hyper::body::Body,
        B::Error: std::fmt::Debug,
    {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn post_query_succeeds() {
        let router = graphql_router(test_schema(), "/graphql");
        let payload = json!({
            "query": "{ hello(name: \"Djangors\") }"
        });
        let req = hyper::Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(Bytes::from(payload.to_string())))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&read_body_string(resp).await).unwrap();
        assert_eq!(body["data"]["hello"], "Hello, Djangors!");
    }

    #[tokio::test]
    async fn post_mutation_with_variables_succeeds() {
        let router = graphql_router(test_schema(), "/graphql");
        let payload = json!({
            "query": "mutation Add($item: String!) { addItem(item: $item) }",
            "operationName": "Add",
            "variables": { "item": "rust" }
        });
        let req = hyper::Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(Bytes::from(payload.to_string())))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&read_body_string(resp).await).unwrap();
        assert_eq!(body["data"]["addItem"], json!(["rust"]));
    }

    #[tokio::test]
    async fn resolver_reads_request_headers() {
        let router = graphql_router(test_schema(), "/graphql");
        let payload = json!({
            "query": "{ authHeader }"
        });
        let req = hyper::Request::builder()
            .method(Method::POST)
            .uri("/graphql")
            .header("authorization", "Bearer secret-token")
            .body(http_body_util::Full::new(Bytes::from(payload.to_string())))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&read_body_string(resp).await).unwrap();
        assert_eq!(body["data"]["authHeader"], "Bearer secret-token");
    }

    #[tokio::test]
    async fn get_graphql_without_query_serves_graphiql_html() {
        let router = graphql_router(test_schema(), "/graphql");
        let req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/graphql")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(content_type.contains("text/html"));
        let body = read_body_string(resp).await;
        assert!(body.contains("GraphiQL"));
    }

    #[tokio::test]
    async fn get_graphql_with_query_executes_query() {
        let router = graphql_router(test_schema(), "/graphql");
        let req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/graphql?query=%7B%20hello%20%7D")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&read_body_string(resp).await).unwrap();
        assert_eq!(body["data"]["hello"], "Hello, world!");
    }

    #[tokio::test]
    async fn get_playground_serves_graphiql_html() {
        let router = graphql_router(test_schema(), "/graphql");
        let req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/graphql/playground")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = read_body_string(resp).await;
        assert!(body.contains("GraphiQL"));
    }

    #[tokio::test]
    async fn execute_graphql_handler_get_missing_query_returns_400() {
        let router = Router::new().get("/graphql", execute_graphql_handler(test_schema()));
        let req = hyper::Request::builder()
            .method(Method::GET)
            .uri("/graphql")
            .body(http_body_util::Full::new(Bytes::new()))
            .unwrap();

        let resp = router.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
