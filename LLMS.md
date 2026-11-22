# Djangors 1.0 — LLM & Agent Guide

Djangors is the batteries-included Django-equivalent web framework for Rust (38 workspace crates,
single-versioned at `1.0.0`).

## Where to Start
- **Single-page cheat sheet**: [`docs/src/guides/ai-agents.md`](docs/src/guides/ai-agents.md)
- **Full CLI reference (`dj`)**: [`docs/src/guides/cli-reference.md`](docs/src/guides/cli-reference.md)
- **Django -> Djangors side-by-side**: [`docs/src/django-comparison.md`](docs/src/django-comparison.md)
- **All topic & how-to guides**: [`docs/src/SUMMARY.md`](docs/src/SUMMARY.md)

## Quick Facts for Coding Agents
1. **Facade crate**: Depend on `djangors = "1.0"` and `djangors-macros = "1.0"`. Opt-in features:
   `mysql`, `sites`, `gis`, `orgs`, `oauth-client`, `oauth-provider`, `saml`, `passkeys`, `ldap`,
   `captcha`, `redis`, `debug-panel`, `otel`, `secrets-remote`.
2. **Models**: Always declare an explicit primary key (`#[djangors(primary_key, auto)] pub id: i64`).
3. **Database**: Pass `&db` (`djangors_db::Database`) explicitly to ORM/auth/migration calls.
4. **Testing**: Run `TEST_BACKEND=sqlite cargo test` locally for instant in-memory SQLite tests, or
   set `DATABASE_URL=postgres://...` for Postgres-only features (FTS, PostGIS, `SKIP LOCKED`, locks).
5. **Code generation**: Prefer `dj generate resource <Name> <field:type...>` and
   `dj openapi --typescript` to avoid hand-writing boilerplate.
