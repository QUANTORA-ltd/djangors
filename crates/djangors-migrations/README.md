# djangors-migrations

Migration engine with autodetection for Djangors

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-migrations = "0.9"
```

Provides schema migration planning, autodetection, execution, and SQL DDL generation for Djangors. It uses runtime enum dispatch across database backends for connection management, topological sorting for migration execution, and ORM type mapping to SQL column types.

- Documentation: https://docs.rs/djangors-migrations
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
