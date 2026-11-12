# djangors

The Django of Rust: batteries-included web framework — ORM, migrations, admin, forms, auth.

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
# Batteries-included (all subsystems + both PostgreSQL and SQLite enabled by default)
djangors = "0.11"

# Or a minimal PostgreSQL-only build (~4.0 MiB release binary):
# djangors = { version = "0.11", default-features = false, features = ["postgres", "rest"] }
```

The main workspace meta-crate bringing together the Djangors web framework components. It re-exports ORM, migrations, admin interface, forms, auth, sessions, views, templates, static files, caching, background tasks, mail, i18n, PDF generation, and contrib modules behind granular Cargo feature flags (`full` by default).

- Documentation: https://docs.rs/djangors
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
