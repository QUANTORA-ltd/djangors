# Multi-tenancy (`djangors-contrib-tenancy`)

`djangors-contrib-tenancy` provides row-level multi-tenancy: a shared database schema where
tenant-scoped tables carry a foreign key to a `Tenant`, and every query against them is
automatically and *verifiably* restricted to the current request's tenant. See
`docs/design/12.1-multi-tenancy.md` in the repository for the full design rationale, including why
this model was chosen over schema-per-tenant or database-per-tenant.

## `Tenant` and `TenantMembership`

```rust,compile
use djangors_contrib_tenancy::{Tenant, TenantMembership};

fn describe(tenant: &Tenant, membership: &TenantMembership) -> String {
    format!("{} ({})", tenant.name, membership.role)
}
```

`Tenant` is deliberately generic (`name`/`slug`/`is_active`). It represents a school, a bank
branch, or a seller account equally well; your application names its own domain concept on top.

`TenantMembership` links a user to a tenant with a role, and a user may hold **more than one**
membership (belonging to several tenants). That's more general than simply putting a `tenant_id`
directly on your user model, and it doesn't require modifying `djangors-auth`'s `User` struct at
all.

## Resolving the current tenant (`TenantResolutionLayer`)

A request's tenant is never trusted from a client-supplied header alone. `TenantResolutionLayer`
reads the `X-Tenant-Id` header, then verifies the authenticated user actually has a real
`TenantMembership` row for that tenant before accepting it. A forged or stale header for a
tenant the user doesn't belong to is silently rejected (no `CurrentTenant` gets set at all, which
downstream scoping then treats as unauthorized), never treated as a default/fallback tenant.

```rust,illustrative
use djangors_contrib_tenancy::TenantResolutionLayer;
use djangors_orm::djangors_db::Database;

fn build_layer(db: Database) -> TenantResolutionLayer<impl Fn(&hyper::Request<hyper::body::Incoming>) -> Option<i64> + Clone> {
    // The extractor closure reads whatever your own upstream auth middleware already stored on
    // the request (e.g. an authenticated user id extension) - this crate doesn't assume a
    // specific auth setup.
    TenantResolutionLayer::new(db, |req| {
        req.extensions().get::<CurrentUserId>().map(|u| u.0)
    })
}

#[derive(Clone, Copy)]
struct CurrentUserId(i64);
```

## Scoping a model to the current tenant

`djangors-rest` already ships the enforcement primitive this crate builds on: the `Scoped` trait,
which `ScopedViewSet<M>` *requires*. A model without a `scope()` implementation simply cannot be
used with `ScopedViewSet` at all, a compile-time guarantee rather than a convention someone has to
remember. `tenant_scope()` is a one-line helper for writing that implementation:

```rust,compile
use djangors_contrib_tenancy::tenant_scope;
use djangors_core::{Request, error::DjangorsError};
use djangors_macros::Model;
use djangors_orm::QuerySet;
use djangors_rest::Scoped;

#[derive(Model, Debug, Clone, sqlx::FromRow)]
#[djangors(app = "myapp", table_name = "myapp_schoolclass")]
struct SchoolClass {
    #[djangors(primary_key, auto)]
    id: i64,
    #[djangors(foreign_key(on_delete = "cascade"))]
    tenant: djangors_orm::ForeignKey<djangors_contrib_tenancy::Tenant>,
    #[djangors(max_length = 100)]
    name: String,
}

impl Scoped for SchoolClass {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        tenant_scope(req, qs, "tenant")
    }
}
```

Every `ScopedViewSet::<SchoolClass>` request now only ever sees rows belonging to the current,
membership-verified tenant. A user for tenant A genuinely cannot see tenant B's classes, list
them, or fetch one by id.

## Scoping the admin to the current tenant

The generated admin UI enforces the same tenant boundary as `ScopedViewSet`. An `AdminSite` is
configured with a `TenantScope`, which names the tenant FK field present on **every** registered
model plus a function that extracts the current tenant id from a request. The extractor typically
reads `djangors_contrib_tenancy::CurrentTenant` from request extensions — which
`TenantResolutionLayer` populates — so the admin and the REST layer resolve the tenant the same
way:

```rust,illustrative
use djangors_contrib_tenancy::CurrentTenant;
use djangors_core::Request;

let site = djangors_admin::AdminSite::new()
    .with_tenant_scoping("tenant", |req: &Request| req.ext::<CurrentTenant>().map(|t| t.0));
```

### The tenant is resolved per request, not stored

This is the important part. A `ModelAdmin` is built **once** at registration and shared across
every concurrent request behind an `Arc`. The tenant restriction is therefore passed as a
`TenantFilter` argument (`Option<(&'static str, i64)>`) into each admin data method — `changelist`,
`export_csv_rows`, `get_by_pk`, `update_from_form`, `update_fields_from_form`, `create_from_form`,
and `delete_by_pk` — and is **never** held on the admin itself.

It must not be stored on the admin: state kept on the shared object could be overwritten by a
second request while the first was awaiting the database, so one tenant's request could read
another tenant's rows. A custom `ModelAdmin` must honour the `tenant` argument it receives rather
than caching a tenant on `self`.

### Superusers are unrestricted

`TenantFilter` is `None` for a superuser, so they see every tenant's rows in the admin.

### It fails closed

For a non-superuser on a tenant-scoped site, if no tenant id can be resolved from the request,
the handler returns `Forbidden`. It does not fall back to running the query unscoped. Failing open
here would silently expose every tenant's rows to any admin user whose request happened to lack a
resolved tenant — the exact leak the scoping exists to prevent.

### What scoping actually guarantees

The trait-level docs make the guarantees precise:

- **Reads and writes by primary key are filtered by tenant too.** A cross-tenant primary key reads
  as "not found" rather than leaking a row (`get_by_pk`), and updates refuse to touch another
  tenant's row.
- **Deletes refuse** to remove a row that belongs to another tenant.
- **Objects created through a scoped add-form have their tenant field forced to the current
  tenant**, so a submitted form cannot plant a row in someone else's tenant.
- **Object history is gated on the object's visibility**, because the log table has no tenant
  column of its own.

### `TenantResolutionFailed`

Previously a database failure during the membership lookup was indistinguishable from "this user
is not a member", so an outage surfaced to clients as `Unauthorized`. The middleware now records a
`TenantResolutionFailed` marker on the request when the lookup itself fails, and `tenant_scope`
maps that to `Internal` (500) instead of `Unauthorized`. For callers who need to name the type,
`TenantResolutionService` is now re-exported from the crate root.

## What this doesn't cover yet

Schema-per-tenant/database-per-tenant support, subdomain-based tenant resolution (v1 is
`X-Tenant-Id`-header only), and automatic Postgres Row-Level Security policy generation are
deliberately out of scope for this first slice.
