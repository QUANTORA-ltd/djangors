# djangors-db

Database backends for the Djangors web framework

Part of [Djangors](https://github.com/QUANTORA-ltd/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
# Both PostgreSQL and SQLite enabled by default:
djangors-db = "0.9"

# Or select a single backend:
# djangors-db = { version = "0.9", default-features = false, features = ["postgres"] }
```

Provides database backends (PostgreSQL and SQLite, each behind its own Cargo feature flag), connection pool management, dialect-aware SQL execution, savepoints, and transaction handling for Djangors built on SQLx.

- Documentation: https://docs.rs/djangors-db
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/QUANTORA-ltd/djangors

Licensed under either of MIT or Apache-2.0 at your option.
