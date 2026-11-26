# djangors-contrib-graphql

GraphQL integration for Djangors powered by [`async-graphql`](https://github.com/async-graphql/async-graphql).

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-graphql = "1.0"
```

Provides `execute_graphql_handler`, `graphiql_handler`, `graphql_router` (with GraphiQL playground support), and `RequestHeaders` context injection for inspecting HTTP headers inside GraphQL resolvers.

- Documentation: https://docs.rs/djangors-contrib-graphql
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
