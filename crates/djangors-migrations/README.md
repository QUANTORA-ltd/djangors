# djangors-migrations

Migration engine with autodetection for Djangors

Part of [Djangors](https://github.com/Chidi09/djangors), a batteries-included web framework for Rust in the spirit of Django.

```toml
[dependencies]
djangors-migrations = "0.6"
```

Provides schema migration planning, autodetection, execution, and SQL DDL generation for Djangors. It uses runtime enum dispatch across database backends for connection management, topological sorting for migration execution, and ORM type mapping to SQL column types.

## Production migrations

Generate migrations with `dj makemigrations`, commit the numbered SQL files in
`migrations/` together with `migrations/.schema_snapshot.json`, and run
`dj migrate` as part of deployment. The migration directory must be present in
the deployed image at the application's working directory; for example, copy it
into the image alongside the application binary. `dj migrate` applies every
unapplied migration file in filename order, including changes made after the
initial migration.

Do not rely on a development machine's uncommitted `migrations/` directory. If
the directory is absent and the database already has `0001_initial`, startup
migration checks registered models against the live schema and returns a clear
error when tables or columns are missing. Generate and deploy the missing
migration instead of allowing requests to fail later with database errors.

- Documentation: https://docs.rs/djangors-migrations
- Guides: https://djangors.vercel.app/docs/
- Repository: https://github.com/Chidi09/djangors

Licensed under either of MIT or Apache-2.0 at your option.
