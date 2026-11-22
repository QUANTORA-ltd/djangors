# Sites Framework (`djangors-contrib-sites`)

`djangors-contrib-sites` mirrors `django.contrib.sites`: it lets a single Djangors application
associate models, URLs, sitemaps, and configuration with one or more domain-backed `Site` records
stored in the `django_site` table.

## Installation

Enable the `sites` feature on `djangors`:

```toml
[dependencies]
djangors = { version = "1.0", features = ["sites"] }
```

## The `Site` Model

`Site` maps to the `django_site` table with three fields:

| Field | Type | Description |
|---|---|---|
| `id` | `i64` | Primary key (`auto`) |
| `domain` | `String` | Unique domain name (e.g. `"example.com"`, max 100 chars) |
| `name` | `String` | Human-readable display name (e.g. `"Example"`, max 50 chars) |

## Middleware & Request Extractor

Mount `CurrentSiteLayer` on your `Router` to resolve the active `Site` from the incoming `Host`
header (with automatic fallback to `DJANGORS_SITE_ID` or the configured default site ID):

```rust,illustrative
use djangors_contrib_sites::{CurrentSite, CurrentSiteLayer, Site};
use djangors_core::{DjangorsError, Request, Response, Router, StatusCode};

async fn home(req: Request) -> Result<Response, DjangorsError> {
    let site = req
        .extensions()
        .get::<CurrentSite>()
        .map(|cs| cs.0.name.clone())
        .unwrap_or_else(|| "Default".to_string());

    Ok(Response::text(StatusCode::OK, format!("Welcome to {site}")))
}

pub fn routes() -> Router {
    Router::new()
        .get("/", home)
        .layer(CurrentSiteLayer::new())
}
```

You can also resolve the current site directly in any handler or service using
`get_current_site(&db, host_header, fallback_site_id).await`.

## Admin Integration

Register `Site` in your admin site with one line:

```rust,illustrative
use djangors_admin::AdminSite;

let mut admin = AdminSite::new();
djangors_contrib_sites::register_admin(&mut admin);
```
