# djangors-admin

Auto-generated admin interface for Djangors

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-admin = "0.9"
```

Provides an automatic database-driven administration interface mirroring Django's Admin. Key components include `AdminSite` as the central registry mounting admin routes and `ModelAdmin` for customizing lists, forms, validation, and actions for registered ORM models with dynamic CSRF-protected template rendering.

- Documentation: https://docs.rs/djangors-admin
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
