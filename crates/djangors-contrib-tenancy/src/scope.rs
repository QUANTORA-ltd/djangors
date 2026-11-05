//! A one-line helper for implementing `djangors_rest::Scoped` on a tenant-scoped model.

use crate::middleware::{CurrentTenant, TenantResolutionFailed};
use djangors_core::error::DjangorsError;
use djangors_core::request::Request;
use djangors_orm::{
    expr::UnresolvedCompare, expr::UnresolvedExpr, expr::Value, FromRow, Model, QuerySet,
};

/// Filters `qs` to only rows where `tenant_field` (the model's foreign-key column to `Tenant`,
/// e.g. `"tenant"`) matches the current request's resolved, membership-verified tenant. Returns
/// `DjangorsError::Internal` if the tenant-membership lookup itself failed (a database error), or
/// `DjangorsError::Unauthorized` if no `CurrentTenant` has been resolved for this request (no
/// `X-Tenant-Id` header, or the header's tenant failed the real membership check in
/// `TenantResolutionLayer` - either way, this is a hard rejection, never a default/fallback
/// tenant).
///
/// Typical usage, one line per tenant-scoped model:
/// ```text
/// impl Scoped for SchoolClass {
///     fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
///         tenant_scope(req, qs, "tenant")
///     }
/// }
/// ```
pub fn tenant_scope<T: Model + FromRow>(
    req: &Request,
    qs: QuerySet<T>,
    tenant_field: &'static str,
) -> Result<QuerySet<T>, DjangorsError> {
    if req.ext::<TenantResolutionFailed>().is_some() {
        return Err(DjangorsError::Internal(
            "tenant membership lookup failed".to_string(),
        ));
    }

    // `CurrentTenant` is set by `TenantResolutionLayer` via `req.extensions_mut()` (a real,
    // per-request tower middleware) - `Router::dispatch` propagates the incoming hyper request's
    // extensions into `Request::ext()`, NOT into `Request::state()` (which is app-wide state set
    // once via `Router::with_state`, per `Request::state`'s own doc comment). Reading via
    // `.state()` here would never see a value a real middleware set, silently rejecting every
    // request - it must be `.ext()`.
    let tenant = req
        .ext::<CurrentTenant>()
        .ok_or_else(|| DjangorsError::Unauthorized("no current tenant on request".into()))?;
    qs.filter(UnresolvedExpr::And(vec![UnresolvedCompare {
        field: tenant_field,
        value: Value::I64(tenant.0),
    }]))
    .map_err(|e| DjangorsError::Internal(e.to_string()))
}
