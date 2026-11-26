# Optional GraphQL & gRPC Adapters

Both `graphql` (`djangors-contrib-graphql`) and `grpc` (`djangors-contrib-grpc`) are opt-in Cargo features on `djangors` so core compile times stay lean unless you enable them.

## GraphQL (`features = ["graphql"]`)

```rust,illustrative
use djangors::graphql::{
    async_graphql::{EmptyMutation, EmptySubscription, Object, Schema},
    graphql_router,
};

struct Query;

#[Object]
impl Query {
    async fn hello(&self) -> &str { "Hello from Djangors GraphQL!" }
}

let schema = Schema::build(Query, EmptyMutation, EmptySubscription).finish();
let app = Router::new().mount("", graphql_router(schema, "/graphql"));
```

`graphql_router` mounts `POST /graphql`, `GET /graphql?query=...`, and serves the interactive GraphiQL playground on `GET /graphql` (without `?query`) and `GET /graphql/playground`.

## gRPC (`features = ["grpc"]`)

`djangors::grpc` integrates `tonic` with `djangors_core::AppState` and provides `HybridServer` to serve HTTP and gRPC either on separate ports (`serve_dual`) or multiplexed on a single port (`serve_multiplexed`):

```rust,illustrative
use djangors::grpc::{AppStateInterceptor, HybridServer};

let interceptor = AppStateInterceptor::new(app_state);
let server = HybridServer::new(http_router, grpc_routes);
server.serve_multiplexed("0.0.0.0:8080".parse()?).await?;
```
