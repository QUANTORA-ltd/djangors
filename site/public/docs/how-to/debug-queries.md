# How to Find Slow and N+1 Queries

## Problem

A page is slow, and you suspect the database: too many queries, a slow one, or the same
query running once per row in a loop (the N+1 problem).

## Solution

Djangors includes a development query panel similar to Django Debug Toolbar or Rails'
Bullet. For every request it records each SQL statement and how long it took, flags
queries that ran more than once, and shows the results:

- in response headers, on every response, including JSON APIs:

  | Header | Value |
  | :--- | :--- |
  | `X-Djangors-Query-Count` | Number of SQL queries the request ran |
  | `X-Djangors-Query-Time-Us` | Total time spent in SQL, in microseconds |
  | `X-Djangors-Request-Time-Us` | Total request time, in microseconds |
  | `X-Djangors-Duplicate-Queries` | Number of distinct queries that ran more than once |

- in a collapsible toolbar at the bottom of full HTML pages, listing every query with
  its timing and a warning for each repeated one.

### 1. Enable the feature

The panel is behind the `debug-panel` Cargo feature, which is off by default:

```toml
[dependencies]
djangors = { version = "0.9", features = ["debug-panel"] }
```

### 2. Wrap your router, in development only

`debug_panel_layer()` is a Tower layer. Wrap your `RouterService` with it and serve
the result with `serve_service`:

```rust,illustrative
use djangors_core::{debug_panel_layer, Djangors, DjangorsSettings, Router};
use djangors_core::router::RouterService;
use tower::Layer;

async fn serve(settings: DjangorsSettings, router: Router) -> Result<(), djangors_core::DjangorsError> {
    let debug = settings.debug;
    let service = RouterService::new(router.clone(), debug);
    let app = Djangors::new(settings, router);
    let listener = app.bind().await?;

    if debug {
        app.serve_service(listener, debug_panel_layer().layer(service)).await
    } else {
        app.serve_service(listener, service).await
    }
}
```

> **Never enable the panel in production.** It shows the SQL text of every query to
> whoever loads the page. The layer doesn't check the `debug` setting itself, so gate
> it on `settings.debug` as above, or only enable the `debug-panel` feature in
> development builds.

For JSON APIs you can keep the headers and skip the HTML toolbar with
`DebugPanelLayer::new().with_html_injection(false)`.

### 3. Read the results

Load the page and look at the toolbar, or check the headers:

```bash
curl -sI http://127.0.0.1:8000/posts/ | grep -i x-djangors
```

A duplicate count above zero usually means a loop that queries once per row. Fetch
the related rows in one query instead, for example with a `filter` on the list of IDs.

## Counting queries in tests

The panel is built on `djangors_db::with_query_log`, which you can call directly. It
runs a future, records every query executed inside it, and returns the statistics. Use
it in a test to make sure a page doesn't grow an N+1 problem later:

```rust,compile
use djangors_db::{with_query_log, Database};

async fn list_posts(db: &Database) {
    // ... the code under test ...
#   let _ = db;
}

async fn check_query_budget(db: &Database) {
    let ((), stats) = with_query_log(list_posts(db)).await;

    assert!(stats.queries.len() <= 2, "too many queries: {:#?}", stats.queries);
    assert!(stats.duplicates.is_empty(), "N+1 loop: {:#?}", stats.duplicates);
}
```

`QueryStats` holds `queries` (each with its `sql`, `param_count`, and
`duration_micros`), `total_duration_micros`, and `duplicates` (each with its `sql`,
`count`, and `total_duration_micros`). Outside `with_query_log`, recording costs
almost nothing: one task-local lookup per query.
