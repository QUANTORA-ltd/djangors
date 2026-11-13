# djangors-contrib-tenancy

Multi-tenancy support for Djangors: a Tenant model, membership, and per-request tenant scoping

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-contrib-tenancy = "0.13"
```

Provides multi-tenancy support for Djangors using a shared-schema, row-level-scoping model. It ships a `Tenant` model, a `TenantMembership` join model, a `TenantResolutionLayer` middleware to resolve per-request tenant context, and a `tenant_scope()` helper for REST scoping.

- Documentation: https://docs.rs/djangors-contrib-tenancy
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
