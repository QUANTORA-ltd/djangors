#![deny(missing_docs)]
//! Multi-site domain resolution for Djangors (mirrors `django.contrib.sites`).
//!
//! Provides the [`Site`] ORM model (`django_site`), [`CurrentSite`] request extension,
//! [`get_current_site`] database lookup helper, [`CurrentSiteLayer`] Tower middleware,
//! and [`register_admin`] admin registration helper.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use djangors_core::DjangorsError;
use djangors_db::Database;
use djangors_macros::Model;
use djangors_orm::{Model, UnresolvedCompare, UnresolvedExpr, Value};
use http_body_util::Full;
use tower::{Layer, Service};

/// A site domain and human-readable display name stored in `django_site`.
#[derive(Model, Debug, Clone, PartialEq, Eq)]
#[djangors(app = "djangors_contrib_sites", table_name = "django_site")]
pub struct Site {
    /// Auto-incrementing primary key.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Domain name (e.g. `"example.com"`).
    #[djangors(max_length = 100, unique)]
    pub domain: String,
    /// Human-readable display name (e.g. `"Example Site"`).
    #[djangors(max_length = 50)]
    pub name: String,
}

/// Request extension holding the resolved [`Site`] for the current HTTP request.
///
/// Inserted by [`CurrentSiteLayer`] and accessible in handlers via
/// `req.ext::<CurrentSite>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSite(pub Site);

/// Strips an optional `:port` suffix from a `Host` header value and lowercases it.
fn normalize_host(host: &str) -> String {
    let trimmed = host.trim();
    let without_port = if trimmed.starts_with('[') {
        if let Some(close) = trimmed.find(']') {
            &trimmed[..=close]
        } else {
            trimmed
        }
    } else {
        trimmed.split(':').next().unwrap_or(trimmed)
    };
    without_port.to_ascii_lowercase()
}

/// Looks up the [`Site`] matching `host` in the database.
///
/// First queries by the normalized domain (lowercased, port stripped). If no row
/// matches and `host` contained a port, falls back to matching the full lowercased
/// `host` string.
pub async fn get_current_site(db: &Database, host: &str) -> Result<Option<Site>, DjangorsError> {
    let normalized = normalize_host(host);
    if normalized.is_empty() {
        return Ok(None);
    }

    let site = Site::objects()
        .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
            field: "domain",
            value: Value::Text(normalized.clone()),
        }]))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .first(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    if site.is_some() {
        return Ok(site);
    }

    let raw_lower = host.trim().to_ascii_lowercase();
    if raw_lower != normalized {
        return Site::objects()
            .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                field: "domain",
                value: Value::Text(raw_lower),
            }]))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .first(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()));
    }

    Ok(None)
}

/// Tower [`Layer`] that resolves the current [`Site`] from the incoming request's
/// `Host` header (or URI authority) and attaches [`CurrentSite`] to the request
/// extensions when a matching row exists.
#[derive(Clone)]
pub struct CurrentSiteLayer {
    db: Database,
}

impl CurrentSiteLayer {
    /// Creates a new [`CurrentSiteLayer`] backed by `db`.
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

impl<S> Layer<S> for CurrentSiteLayer {
    type Service = CurrentSiteService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CurrentSiteService {
            inner,
            db: self.db.clone(),
        }
    }
}

/// Service wrapper produced by [`CurrentSiteLayer`].
#[derive(Clone)]
pub struct CurrentSiteService<S> {
    inner: S,
    db: Database,
}

impl<S, B> Service<hyper::Request<B>> for CurrentSiteService<S>
where
    S: Service<hyper::Request<B>, Response = hyper::Response<Full<Bytes>>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    B: Send + 'static,
{
    type Response = hyper::Response<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: hyper::Request<B>) -> Self::Future {
        let mut inner = self.inner.clone();
        let db = self.db.clone();

        let host = req
            .headers()
            .get(hyper::header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or_else(|| req.uri().authority().map(|a| a.as_str().to_owned()));

        Box::pin(async move {
            if let Some(host_str) = host {
                if let Ok(Some(site)) = get_current_site(&db, &host_str).await {
                    req.extensions_mut().insert(CurrentSite(site));
                }
            }
            inner.call(req).await
        })
    }
}

/// Registers the [`Site`] model with a `djangors-admin` [`AdminSite`](djangors_admin::AdminSite).
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<Site>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::{service_fn, ServiceBuilder, ServiceExt};

    async fn create_site_table(db: &djangors_test::TestDatabase) {
        let auto_pk = db.database().dialect().auto_pk_type();
        db.create_table(&format!(
            "CREATE TABLE IF NOT EXISTS django_site (\
             id {auto_pk}, \
             domain VARCHAR(100) UNIQUE NOT NULL, \
             name VARCHAR(50) NOT NULL)"
        ))
        .await
        .unwrap();
    }

    #[test]
    fn site_model_uses_django_site_table() {
        assert_eq!(Site::meta().table_name, "django_site");
    }

    #[tokio::test]
    async fn get_current_site_resolves_domain_with_and_without_port() {
        let db = djangors_test::TestDatabase::isolated().await.unwrap();
        create_site_table(&db).await;

        Site {
            id: 0,
            domain: "example.com".to_string(),
            name: "Example".to_string(),
        }
        .save(db.database())
        .await
        .unwrap();

        let exact = get_current_site(db.database(), "example.com")
            .await
            .unwrap()
            .expect("should match exact domain");
        assert_eq!(exact.name, "Example");

        let with_port = get_current_site(db.database(), "EXAMPLE.COM:8080")
            .await
            .unwrap()
            .expect("should match case-insensitively with port stripped");
        assert_eq!(with_port.domain, "example.com");

        let missing = get_current_site(db.database(), "unknown.org")
            .await
            .unwrap();
        assert!(missing.is_none());

        db.cleanup().await.unwrap();
    }

    #[tokio::test]
    async fn current_site_layer_injects_extension() {
        let db = djangors_test::TestDatabase::isolated().await.unwrap();
        create_site_table(&db).await;

        Site {
            id: 0,
            domain: "tenant.local".to_string(),
            name: "Tenant Portal".to_string(),
        }
        .save(db.database())
        .await
        .unwrap();

        let svc_fn = service_fn(|req: hyper::Request<Full<Bytes>>| async move {
            let name = req
                .extensions()
                .get::<CurrentSite>()
                .map(|cs| cs.0.name.clone())
                .unwrap_or_else(|| "none".to_string());
            let resp = hyper::Response::builder()
                .status(200)
                .body(Full::new(Bytes::from(name)))
                .unwrap();
            Ok::<_, Infallible>(resp)
        });

        let mut svc = ServiceBuilder::new()
            .layer(CurrentSiteLayer::new(db.database().clone()))
            .service(svc_fn);

        let req = hyper::Request::builder()
            .uri("/")
            .header("host", "tenant.local:8000")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        let body = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(body.as_ref(), b"Tenant Portal");

        let req_unknown = hyper::Request::builder()
            .uri("/")
            .header("host", "other.local")
            .body(Full::new(Bytes::new()))
            .unwrap();
        let resp_unknown = svc.ready().await.unwrap().call(req_unknown).await.unwrap();
        let body_unknown = http_body_util::BodyExt::collect(resp_unknown.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(body_unknown.as_ref(), b"none");

        db.cleanup().await.unwrap();
    }

    #[test]
    fn register_admin_registers_site_model() {
        let admin = djangors_admin::AdminSite::new();
        register_admin(&admin);
    }
}
