# djangors-db

Database backends for the Djangors web framework

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-db = "0.6"
```

Provides database backends, connection pool management, and transaction handling for Djangors built on SQLx. It supports configuration-driven connection pooling and explicit transaction isolation levels across database engines.

- Documentation: https://docs.rs/djangors-db
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.

## Query observability

Every query sent through `Database::conn()` records a `db.query` tracing span with
the redacted statement, inferred table, returned or affected row count, duration,
and success flag. Bound values are not recorded. Configure slow-query warnings
and an observer to export the same data to an application metrics system:

```rust,ignore
use std::time::Duration;
use djangors_db::{Database, DatabaseConfig};

let database = Database::connect(&DatabaseConfig::new(database_url)).await?;
let database = database
    .with_slow_query_threshold(Duration::from_millis(100))
    .with_query_observer(|event| {
        if let Some(table) = &event.model {
            // Record `event.duration` and `event.rows` with your metrics client.
            tracing::debug!(table, elapsed = ?event.duration, rows = ?event.rows);
        }
    });
```

The observer runs after each `Conn::fetch_all`, `fetch_one`, `fetch_optional`, or
`execute` call. Observer panics are contained and do not affect the query result.
