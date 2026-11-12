# Changelog

All notable changes to Djangors are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added

- **API keys** (`djangors_rest::api_keys`): keys for machines, shaped `<prefix>.<secret>`.
  Only a SHA-256 hash of the secret is stored, and the full key is shown once.
  `ApiKeyAuth` reads `X-Api-Key` or `Authorization: Api-Key <key>` and compares in constant
  time; every failure returns the same 401. Scopes, expiry, `revoke_api_key`,
  `HasApiKey` / `HasApiKeyScope` permissions, an admin page (the facade's `admin` feature),
  and `OpenApiBuilder::api_key_security`. See
  [How to Authenticate Machines with API Keys](docs/src/how-to/api-keys.md).
- **CORS**: `CorsConfig` and `cors_layer` over `tower-http`'s CORS layer, with
  `CorsConfig::from_settings`. Origins come from `cors_allowed_origins` in `djangors.toml`
  or `DJANGORS_CORS_ALLOWED_ORIGINS`. `"*"` with credentials and origins with a path are
  rejected, and `dj check --deploy` warns about `"*"` in production.
- **API versioning** (`djangors_core::versioning`): read the version from the URL prefix
  (`/v2/...`), a header, or an `Accept` parameter with the `ApiVersion` extractor;
  unsupported versions get a 400 problem response. `DeprecationLayer` adds `Deprecation`
  and `Sunset` headers. `OpenApiBuilder::version`.
- **Health checks**: `Router::health(HealthChecks::new().with(DatabaseCheck::new(db)))`
  adds `/healthz` (liveness) and `/readyz` (runs checks concurrently with a timeout,
  503 on failure). `CacheCheck` for cache backends. New projects from `dj new` use it.
- **Password validation** (`djangors_auth::password_validation`): minimum length,
  all-digits, similarity to the username or email, and zxcvbn strength scoring
  (`password-strength` feature, on by default). Used by `confirm_password_reset` and
  `dj createsuperuser` (`--skip-validation` to bypass).
- **Resilience** (`djangors_core::resilience`): `retry` with exponential backoff and
  jitter (`resilience` feature, on by default), `with_timeout`, and a `CircuitBreaker`.
- **Distributed locks**: `djangors_db::locks::with_lock` / `with_lock_wait` hold a
  Postgres advisory lock for the length of a transaction, so it can never leak onto a
  pooled connection (a per-process lock on SQLite). `RedisLock` (cache `redis` feature)
  releases only while it still owns the key.
- **Transactional outbox**: `djangors_tasks::enqueue_in` / `enqueue_scheduled_in` queue a
  task inside the caller's transaction, so the task exists only if the data change
  commits.

### Changed

- `AuthError` is now `#[non_exhaustive]` and has a new `WeakPassword(Vec<String>)`
  variant. Code that matches every `AuthError` variant needs a wildcard arm.
- `confirm_password_reset` rejects weak passwords with `AuthError::WeakPassword`.
- `dj new` projects serve `/healthz` and `/readyz` through `Router::health` instead of a
  hand-written `healthz` view.

## [0.11.0]

There is no 0.10.0 release: its features ship here together with 0.11. New: idempotency keys,
htmx helpers, passkeys, soft deletes, encrypted fields, model history, Postgres-backed groups,
`dj seed`, OpenTelemetry export, and a request body limit.

### Added

- **Idempotency keys** (`djangors_cache::IdempotencyLayer`): a `POST`/`PUT`/`PATCH`/`DELETE`
  with an `Idempotency-Key` header runs once; retries get the stored response
  (`Idempotent-Replayed: true`), concurrent duplicates get 409, and a reused key with a
  different body gets 422. Keys are scoped per caller (`Authorization` + `Cookie` by default,
  or `with_scope`). A request abandoned mid-flight releases its key. See
  [How to Make Payments and Other POSTs Safe to Retry](docs/src/how-to/idempotency.md).
- `Cache::add`, an atomic store-if-absent operation, implemented for the in-memory,
  database, and Redis backends. Custom `Cache` implementations get a default that returns
  `CacheError::Unsupported`.
- **htmx helpers**: `Request::is_htmx`, `is_htmx_boosted`, `htmx_target`, `htmx_trigger`,
  `htmx_current_url`, and `Response::hx_trigger`, `hx_redirect`, `hx_refresh`,
  `hx_push_url`, `hx_retarget`, `hx_reswap`. See [How to Use htmx](docs/src/how-to/htmx.md).
- `dj openapi --api-prefix <PREFIX>` and repeatable `--model <NAME>`; the output states
  the path pattern it assumes.
- `DebugPanelLayer::for_settings(&settings)`, which turns the panel off entirely when
  `debug` is false.
- **Passkeys** (`passkeys` feature, `djangors_auth::passkeys`): WebAuthn registration and
  sign-in with ES256 and Ed25519, implemented in pure Rust (no OpenSSL). User verification
  is required, the signature counter detects cloned authenticators, and `PasskeyStore`
  keeps passkeys in `djangors_auth_passkey` on PostgreSQL and SQLite. See
  [Passkeys](docs/src/guides/passkeys.md).
- **Soft deletes**: `#[djangors(soft_delete)]` on a model with a nullable `deleted_at`
  field. Deleting sets `deleted_at`, and queries hide deleted rows. Adds
  `QuerySet::with_deleted`, `only_deleted`, `force_delete`, and instance `restore` and
  `force_delete`. See the [ORM guide](docs/src/guides/orm.md#soft-deletes).
- **Encrypted fields**: `#[djangors(encrypted)]` on `String` and `Option<String>` fields
  encrypts values with AES-256-GCM on write and decrypts them on load. The key comes from
  `DJANGORS_FIELD_ENCRYPTION_KEY`; `DJANGORS_FIELD_ENCRYPTION_OLD_KEYS` allows rotation.
  Filtering and ordering on encrypted fields return an error. See
  [Encrypted Fields](docs/src/guides/encrypted-fields.md).
- **Model history** (`djangors-contrib-history`, `history` feature, on by default):
  `track::<M>()` records every create, update and delete of a model as a JSON snapshot;
  `history_for` reads it back. See [Model History](docs/src/guides/model-history.md).
- **`PgGroups`**: broadcasts WebSocket and SSE group messages across processes with
  PostgreSQL `LISTEN`/`NOTIFY`, so several servers behind a load balancer share groups.
- **`dj seed`** runs the project's `src/bin/seed.rs`. It refuses to run when `DJANGORS_DEBUG` is
  false unless `--force` is given. `dj new` scaffolds the seed script.
- **OpenTelemetry** (`otel` feature): `logging::init_otel_logging(service_name)` exports
  tracing spans over OTLP/HTTP. The router emits a `request` span (method, path) and
  `Conn` emits `sql` spans with the statement text, never bound values. See
  [How to Export Traces with OpenTelemetry](docs/src/how-to/opentelemetry.md).
- **Request body limit**: requests larger than `DjangorsSettings::max_body_bytes`
  (10 MiB by default; `max_body_bytes` in `djangors.toml` or `DJANGORS_MAX_BODY_BYTES`) get `413 Payload Too Large` before the
  handler runs. `Router::max_body_bytes` overrides it per router.
  `IdempotencyLayer::max_body_bytes` limits the bodies that layer buffers (also 10 MiB).

### Changed

- `RouterService` accepts any request body type, so body-buffering middleware such as
  `IdempotencyLayer` can wrap it.
- `QuerySet::delete` and instance `delete` on soft-delete models set `deleted_at` instead
  of removing rows. Other models are unchanged.
- **Breaking for tests:** `#[derive(Factory)]` `create()` / `create_many()` return
  `OrmError::InvalidQuery` when a `ForeignKey` / `OneToOne` field was not set with
  `.with_<field>_id(..)`, instead of saving a reference to id 1. `build()` is unchanged.
- A debug panel built without settings logs a warning unless `DJANGORS_DEBUG` is set.

### Fixed

- Internal errors are logged at `error` level (method, path, message) before their text
  is redacted from the response.
- `save()` on a model whose only field is an auto primary key generated invalid SQL
  (`INSERT INTO t () VALUES ()`); it now uses `INSERT INTO t DEFAULT VALUES`.

## [0.9.0]

New convenience features: resource generators, a TypeScript client, model factories,
`dj inspectdb`, a dev-only query panel, WebSockets with live reload, and RFC 9457 error bodies.
The `ws` feature is on by default; builds using `default-features = false` are unaffected.

### Added

- **Standard RFC 9457 (`application/problem+json`) error bodies**:
  - `djangors_core::ProblemDetailsRenderer` and `DjangorsError::into_problem_response()` /
    `DjangorsError::write_problem_json()` emit standard [RFC 9457](https://www.rfc-editor.org/rfc/rfc9457)
    `application/problem+json` error payloads (`type`, `title`, `status`, `code`, `detail`,
    `instance`, `errors`, `details`) using the direct JSON byte writer (`write_json_str` /
    `write_json_i64`).
  - `DjangorsError` now implements `From<FormErrors>` (422 `validation_error` with field map) and
    `From<AuthError>` (`rate_limited` -> 429, `account_locked` -> 429 with `retry_after_secs`,
    `invalid_token` -> 401), unifying REST (`ValidationErrors`), form, and authentication failures
    under a single error shape.
  - Content negotiation automatically returns `application/problem+json` when requested via
    `Accept: application/problem+json`.
- **Typed TypeScript client generation (`dj openapi --typescript`)**:
  - `djangors_rest::openapi::generate_typescript_client` and `OpenApiBuilder::to_typescript()`
    generate TypeScript interfaces (`<Model>`, `<Model>CreateInput`, `<Model>UpdateInput`,
    `PaginatedList<T>`), RFC 9457 `ProblemDetails` / `ProblemDetailsError`, and a zero-dependency
    `DjangorsApiClient` class with typed `list`, `retrieve`, `create`, `update`, `partialUpdate`,
    and `destroy` methods.
  - `dj openapi [--typescript] [--output <path>] [--title <title>] [--api-version <ver>]` emits
    either the OpenAPI 3.0.3 JSON spec or the full TypeScript SDK from compiled model metadata.
- **Resource generators (`dj generate resource`)**:
  - `dj generate resource <Name> <field:type>... [--app <app>]` scaffolds a full-stack resource module
    (`models.rs`, `serializers.rs`, `views.rs`, `admin.rs`, `tests.rs`, `mod.rs`) and a numbered
    reversible SQL migration (`migrations/<NNNN>_create_<table>.sql`).
- **Database schema introspection (`dj inspectdb`)**:
  - `dj inspectdb [--database-url <url>] [--output <path>] [--app <app>]` introspects an existing
    PostgreSQL or SQLite database and emits ready-to-compile `#[derive(Model)]` structs with primary
    keys, nullability (`Option<T>`), and `max_length` attributes.
- **Model factories (`#[derive(Factory)]`)**:
  - `djangors_macros::Factory` (re-exported by `djangors_test::Factory`) generates a `<Model>Factory`
    builder and `Model::factory()` with monotonic sequence-backed defaults per field type, fluent
    `.with_<field>(val)` overrides, `.build()`, `.create(&db)`, and `.create_many(&db, count)`.
- **Dev-only request & SQL query panel (`debug-panel` feature)**:
  - `djangors_db::with_query_log` records per-request SQL statements, parameter counts, and microsecond
    timings via a zero-overhead task-local collector, automatically detecting duplicate N+1 queries
    executed in a loop (`QueryStats::duplicates`).
  - `djangors_core::debug_panel_layer()` (`DebugPanelLayer`, behind the opt-in `debug-panel` feature)
    attaches `X-Djangors-Query-Count`, `X-Djangors-Query-Time-Us`, `X-Djangors-Request-Time-Us`, and
    `X-Djangors-Duplicate-Queries` headers and injects a collapsible HTML query toolbar with N+1 loop
    warnings before `</body>`.
- **RFC 6455 WebSockets (`ws` feature) and live reload in `dj run`**:
  - `djangors_core::ws` (`WebSocket`, `WsMessage`, `WsHandler`, `upgrade_websocket`, and
    `Router::ws(path, handler)`) provides full-duplex RFC 6455 WebSocket upgrades and framing over
    Hyper's HTTP upgrade mechanism.
  - `dj run` now hosts a companion WebSocket live-reload server and injects a lightweight client
    script into `Response::html` responses in dev mode so connected browsers automatically refresh
    whenever Rust sources or templates rebuild.
- **Cacheable view responses**: `Response::with_extension`, `Response::cacheable()`,
  `Response::extensions` / `extensions_mut`, and `CacheService` is now generic over the request
  body, so `CacheLayer` wraps `RouterService` directly.
- **`OriginPolicy`** and `Router::ws_with_origin_policy` to allow cross-origin WebSocket
  handshakes explicitly.

### Changed

- **WebSocket handshakes from another browser origin are rejected (403)** by `Router::ws` and
  `upgrade_websocket`, preventing cross-site WebSocket hijacking with the user's cookies.
  Handshakes must send `Sec-WebSocket-Version: 13`.
- **Internal error text is no longer sent to JSON / problem+json clients outside debug mode.**
  `DjangorsError::Internal` and `Panicked` render as "Internal Server Error" when `debug` is off,
  matching the HTML production page. `ProblemDetailsRenderer` always redacts them.
- **`DateTime<Utc>` model fields read Postgres `TIMESTAMP WITHOUT TIME ZONE` columns** (as UTC)
  as well as `TIMESTAMPTZ`.
- `dj generate resource` writes SQLite column types when `DATABASE_URL` is a SQLite URL and prints
  the wiring steps (module, dependencies, routes, admin) after generating.
- `dj inspectdb` warns on stderr for column types Djangors cannot map (json, bytea, inet, arrays,
  intervals), no longer maps `INTERVAL` / `POINT` to `i32`, and emits `column = "..."` for column
  names that are not valid Rust identifiers.

### Fixed

- WebSocket frames: fragmented messages are reassembled, unmasked client frames, invalid control
  frames, stray continuations and unknown opcodes close the connection with status 1002, and a
  handshake on a connection that cannot upgrade returns 400 instead of a 101 that never completes.
- The live-reload script and debug panel are injected only into full pages (`</body>`), not HTML
  fragments such as htmx partials.
- Clippy on the CI toolchain (1.96) no longer fails on a lint name that only newer clippy knows.

## [0.8.0]

This release contains **breaking changes** (`#[non_exhaustive]` on three public structs), so per
the pre-1.0 policy in [API Stability](docs/src/api-stability.md) it bumps the minor version.

### Migrating from 0.7.0

`DjangorsSettings`, `djangors_mail::Message`, and `djangors_rest::ViewSetConfig` are now
`#[non_exhaustive]` so future fields can be added without requiring downstream struct-literal
edits. Construct them with the provided constructors and builders:

```rust
let settings = DjangorsSettings::new()
    .with_host("127.0.0.1")
    .with_port(8000)
    .with_debug(false)
    .with_allowed_hosts(["example.com"])
    .with_secret_key(secret_key);

let message = djangors_mail::Message::new(
    ["person@example.com"],
    "sender@example.com",
    "Welcome",
    "Hello",
)
.cc("copy@example.com")
.reply_to("support@example.com");

static VIEW_CONFIG: djangors_rest::ViewSetConfig = djangors_rest::ViewSetConfig::new()
    .with_filterable_fields(&["status"])
    .with_page_size(Some(50));
```

Generated model `save`, `update`, and `delete` methods take a shared `&Database`; pass the
database reference directly (`model.save(db).await?`). They are concrete database-backed methods,
not generic over a transaction executor, so code that previously passed `&&Database` or a
transaction wrapper should use the equivalent API available on that wrapper.

### Added

- **Facade Cargo feature flags (`djangors`)**: `crates/djangors` now exposes granular Cargo
  feature flags (`postgres`, `sqlite`, `migrations`, `rest`, `admin`, `auth`, `forms`, `sessions`,
  `template`, `staticfiles`, `s3`, `cache`, `mail`, `i18n`, `tasks`, `pdf`, `payments`, and
  `tenancy`). All features remain enabled by default so existing applications compile unchanged,
  while minimal builds (`default-features = false, features = ["postgres"]` or `["sqlite"]`)
  drop optional subsystems cleanly.
- **Database backend feature flags (`postgres` and `sqlite`)**: `djangors-db` and downstream
  framework crates now gate PostgreSQL (`sqlx/postgres`) and SQLite (`sqlx/sqlite`) behind
  `postgres` and `sqlite` Cargo features (both on by default). Building with only `postgres`
  excludes `sqlx-sqlite` and `libsqlite3-sys` from the dependency graph; building with only
  `sqlite` excludes `sqlx-postgres`. Compiling `djangors-db` with `--no-default-features` and
  neither backend enabled emits a clear `compile_error!`.
- **Optional `s3` feature on `djangors-staticfiles`** (on by default) and **optional `cache`
  feature on `djangors-core`** (on by default, gating `djangors_core::ratelimit`).
- **Compile-time direct JSON serialization (`fast-json-stringify` style)**:
  - `#[derive(Model)]` generates `Model::write_json` and `Model::write_json_filtered` with
    pre-baked static JSON key slices (`concat!`) sorted lexicographically to match `serde_json::Map`
    key order, streaming field values directly into a `Vec<u8>` buffer via `djangors_orm::json`
    without intermediate `Vec<(&'static str, Value)>` or `serde_json::Value` allocations.
  - `djangors_core::Response::json_bytes(status, Vec<u8>)` constructs an `application/json`
    response directly from pre-serialized bytes without copying.
  - `djangors_rest` adds `serialize_to_vec`, `serialize_many_to_vec`, `Serializer::write_json`,
    `Serializer::write_json_many`, `Serializer::to_json_bytes`, `Pagination::write_envelope`, and
    `CursorPagination::write_envelope_with_cursor`, and wires `ViewSet` / `ScopedViewSet`
    (`list`, `retrieve`, `create`, `update`) to stream JSON responses directly into byte buffers.
- **Release profile in workspace and `dj new`**: Both the workspace root `Cargo.toml` and projects
  scaffolded by `dj new` now include `[profile.release]` with `lto = "thin"`, `strip = "symbols"`,
  and `codegen-units = 1`. Added `examples/minimal-facade` (4.00 MiB stripped release binary with
  `default-features = false, features = ["postgres"]` vs 11.51 MiB for `examples/polls`) with a
  6 MiB CI binary size gate on `minimal-facade` and a 14 MiB gate on `polls`.

### Changed

- **`djangors-mail` now uses `rustls` (`lettre/tokio1-rustls-tls`)** instead of `native-tls`,
  unifying the entire workspace on a single TLS stack (`rustls`) with no OpenSSL/`native-tls`
  dependency.

## [0.7.0]

This release contains a **breaking change to the `ModelAdmin` trait**, made to fix a
cross-tenant data leak in the admin. Per the pre-1.0 policy in
[API Stability](docs/src/api-stability.md), that bumps the minor version.

### Removed

- **`ModelAdmin::set_tenant_filter` and `ModelAdmin::clear_tenant_filter`.** These stored the
  active tenant *on the admin object*. A `ModelAdmin` is constructed once at registration and
  shared across every concurrent request behind an `Arc`, so a handler that set the filter and
  then awaited the database could have a second request overwrite it mid-flight and run the
  first request's query against the wrong tenant. `clear_tenant_filter` was worse: a superuser
  request removed the restriction globally for whatever else was in flight.

  There is no replacement, because the tenant is no longer stored anywhere. See below.

### Changed (breaking)

- **The seven `ModelAdmin` methods that touch data now take a final `tenant: TenantFilter`
  argument**: `changelist`, `export_csv_rows`, `get_by_pk`, `update_from_form`,
  `update_fields_from_form`, `create_from_form`, and `delete_by_pk`. `TenantFilter` is
  `Option<(&'static str, i64)>` — the tenant FK field name plus the id every row must match, or
  `None` for unrestricted.

  **Migrating a custom `ModelAdmin`**: add the parameter to each of those methods and apply it to
  every query the method runs. Do not cache it on `self` — that is the bug this change fixes.
  Implementations that ignore tenancy entirely can accept the argument and ignore it, but will
  then not be tenant-safe. `DefaultModelAdmin` needs no changes from callers.

- **Admin tenant resolution now fails closed.** For a non-superuser on a tenant-scoped site, a
  request whose tenant cannot be resolved returns `Forbidden` instead of running the query
  unscoped across every tenant.

- **`tenant_scope` returns `Internal` rather than `Unauthorized` when the membership lookup
  itself fails.** A database error during tenant resolution was previously indistinguishable
  from "this user is not a member", so an outage was reported to clients as an authorization
  failure. `TenantResolutionLayer` now records a `TenantResolutionFailed` marker that
  `tenant_scope` maps to a 500.

### Added

- **Bachs payment provider** (`djangors-contrib-payments`): `BachsProvider` implementing
  `PaymentProvider` against the [bachs.io](https://bachs.io) REST API — product-less checkout
  sessions, checkout-based verification, refunds, and HMAC-SHA256 webhook verification.
  Note that `verify`/`refund` take the Bachs **checkout id** (returned as `access_code` from
  `initiate`), not your merchant reference, because Bachs has no lookup-by-reference endpoint.
  Not yet exercised against the live API.
- **`PaymentProvider::verify_webhook_signature_with_timestamp`**, for providers that sign over a
  timestamp plus the body. Has a default implementation that ignores the timestamp and delegates
  to `verify_webhook_signature`, so existing implementations (including Paystack) are unaffected.
- **`Router::set_error_renderer(renderer)`**, which performs the `Arc<dyn ErrorRenderer>` cast
  for you. `with_state(Arc::new(MyRenderer))` compiles but stores the value under its concrete
  type, where the renderer lookup never finds it — the custom renderer is then silently ignored.
- **`TenantResolutionService`** is now re-exported from `djangors-contrib-tenancy`'s root,
  alongside the new `TenantResolutionFailed`.
- **`TenantFilter`** is exported from `djangors-admin`.

### Fixed

- **`record_charge_initiated` is now idempotent on SQLite.** It documents that a UNIQUE violation
  on `reference` means "already recorded" and returns the existing row, but it matched only
  Postgres's `23505`. On SQLite — a supported backend since 0.5.0 — a concurrent webhook
  redelivery returned a hard error instead. SQLite's `2067` and `1555` are now recognised too.
- **`admin_history` had no permission check at all**, and no tenant restriction. It now requires
  the `view` permission and confirms the target object is visible to the current tenant before
  reading its audit log (the log table has no tenant column of its own, so visibility is gated on
  the object).
- Several tests only passed on the backend they were written against, hiding the bug above: raw
  `sqlx` against `Database::pool()` (which panics on a SQLite handle), `BIGSERIAL`/`TIMESTAMPTZ`
  DDL, `$1` placeholders, an unquoted `user` column (a reserved word in Postgres), and one
  hardcoded `postgres://…@localhost/djangors_test` that ignored both `TEST_BACKEND` and
  `DATABASE_URL`. The suite now passes on both backends.
- Four ORM tests shared one table against a single Postgres database and raced on `DROP`/`CREATE`
  under parallel execution; they now use isolated per-test databases.

### Internal

- `djangors-admin`'s `lib.rs` went from 9,848 lines to 86, split into twelve modules
  (`model_admin`, `site`, `changelist`, `edit_view`, `delete_view`, `index_view`, `history_view`,
  `action_view`, `forms`, `log`, `perms`, `favicon`). Purely a move — every previously-public item
  is re-exported at its original path, so this is not a breaking change.
- All `#[cfg(test)]` modules across the workspace moved out of production sources into sibling
  test files (59 files). They remain child modules, so they keep access to private items.

### Documentation

- New guidance in `guides/testing.md` on where test code lives and how to write tests that pass
  on both backends; `guides/admin.md` gains the crate layout and the custom-`ModelAdmin` tenancy
  contract. `guides/payments.md`, `guides/multi-tenancy.md`, `guides/http-and-middleware.md`,
  `guides/orm.md`, and `guides/migrations.md` updated for the changes above, plus the previously
  undocumented ORM features (`OneToOne`, `ManyToMany`, custom managers, composite indexes,
  `QuerySet::delete`, query caching, `detect_changes`).

## [0.6.3]

### Router
- **Fix**: `:name`-style path segments (Express/Django URLconf syntax) are now accepted as an alias
  for the untyped `{name}` capture. Previously `parse_pattern` only recognized `{name}`/`{name:type}`
  as a capture; a segment written `:name` was stored as the literal string `":name"`, which a real
  request path segment can never equal — every route registered with that syntax 404'd
  unconditionally, silently. `{name}`/`{name:i64}`/`{name:slug}` remain the recommended forms (they
  give request-time validation the alias doesn't); `:name` is a compatibility shim for code migrated
  from Express/Django-style URLconfs.

### REST
- **Fix**: `scoped_viewset_routes_with_config`'s `GET` list handler was missing the `IsAuthenticated`
  check that its own `create`/`retrieve`/`update`/`destroy` routes already had (and that the sibling
  `viewset_routes_with_config_and_permission` list handler already had) — an unauthenticated `GET /`
  reached `Scoped::scope` directly instead of being rejected up front. Every route this helper mounts
  now requires authentication before running.
- **Docs**: `ViewSet` and `ScopedViewSet` now carry an explicit "No built-in permission check" section
  warning that their associated functions (`list`/`list_with_config`, `retrieve`, `create`, `update`,
  `destroy`, and the `_with_options` variants) perform no authentication or authorization check of
  their own — that's intentional, so callers can compose any `Permission` (including `AllowAny`) at
  the mounting layer — but it means registering one of these functions as a bare route handler
  (`router.post("/x", ScopedViewSet::<M>::create)`) mounts a completely unauthenticated endpoint, with
  `Scoped::scope`'s row filter as the only remaining check (and most `scope` implementations only
  constrain *which tenant's rows*, not *which role*). Always mount through `viewset_routes*` /
  `scoped_viewset_routes*`, which wrap every handler in the permission check for you.

## [0.6.2]

### ORM
- `QuerySet::get_or_create` / `QuerySet::update_or_create` — idempotent upsert helpers; the
  `defaults`/`updates` closures return `Vec<(&'static str, Value)>` / `Vec<(&'static str, SetExpr)>`
  (the `set!` form). Requires `T: Send`; not yet wrapped in a transaction.
- `QuerySet::search(query, &fields)` — Postgres `to_tsvector`/`plainto_tsquery` full-text search that
  chains into the query as a `search_condition`.
- `QuerySet::explain()` — Postgres-only `EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)`, returns the plan
  as a `String`; errors with `UnsupportedOnDialect` on SQLite.
- `FuncExpr` (`aggregate.rs`) — `COALESCE`/`LOWER`/`UPPER`/`CONCAT`/`LENGTH` scalar functions usable
  with `QuerySet::annotate_funcs`.
- `Exists`/`OuterRef` correlated subqueries, `select_for_update` + savepoints (15.1).

### Model macros (`#[derive(Model)]`)
- `#[djangors(auto_now_add = true)]` / `#[djangors(auto_now = true)]` on `DateTime<Utc>` fields —
  auto-stamp `chrono::Utc::now()` on `save()` (both) and `update()` (`auto_now` only).
- `#[djangors(choices = ["a", "b"])]` on `String` fields — populates `FieldMeta.choices`.
- Unblocked `uuid::Uuid`, `chrono::NaiveDate`, `chrono::NaiveTime`, `std::time::Duration`, and
  `rust_decimal::Decimal` field types (persisted via text serialization; `Decimal` requires
  `max_digits`/`decimal_places`).

### Migrations
- `Operation::CreateTable` now carries `check_constraints`; `build_create_all_plan` emits a
  `CHECK (col IN (...))` constraint for every field declared with `#[djangors(choices = [...])]`.
- Migration CLI verbs (`dj migrate`/`makemigrations`/`--rollback`), dialect-aware execution.

### Admin
- **Inlines** — `ModelAdminConfig { inlines: Some(&[InlineConfig { struct_name, relation_field, fields }]) }`
  renders a child model's rows inside the parent's add/change form.
- **Tenant scoping** — `AdminSite::with_tenant_scoping(tenant_field, extract_tenant_id)` filters
  every changelist/add/change/delete query by the resolved tenant; inlines inherit the parent's scope.
- `list_filter` accepts Boolean fields **and** fields declared with `#[djangors(choices = [...])]`.

### REST
- `scoped_viewset_routes_with_config::<M>(router, path, config)` — scoped routes with custom
  `ViewSetConfig` (previously only default config was possible).
- `request.user()` convenience — returns `Result<User, DjangorsError>` (wraps `current_user`).

### Other
- `djangors-views`: named routes + `reverse()`; template functions.
- Multipart fuzz target and updated threat model (security).
- SQLite runs in CI; five Postgres-only fake "passes" became honest skips.
- Docs: new guides for class-based views, flatpages & redirects, messages, object-level permissions,
  sites & 2FA, HTTP & middleware; augmented existing guides; full crate coverage in
  `docs/src/django-comparison.md`.

## [0.6.0 and earlier]

The sections below group the history by the `PLAN.md` phase each change shipped under; the
`[0.6.0]` heading lower in the file marks the first versioned crates.io release since this phased
history was written.

Everything to date, through Phase 12. Workspace version is `0.2.0`; most crates were first
published to crates.io as `0.1.0`, but that snapshot predates every fix and feature in Phase 12
below. See Phase 12's own publish-status note.

### Phase 0-1: Bootstrap and core request/response layer
- Workspace scaffold: `djangors`, `djangors-core`, `djangors-cli` (`dj` binary), and every
  placeholder crate.
- `djangors-core`: `Request`/`Response`/`Router`/`Handler`, tower middleware layer, a real TCP
  server (`Djangors` app struct), typed extractors (`Json`, `Query`, `Form`), panic isolation with
  Django-style debug error pages, a typed async signals bus (`request_started`/`request_finished`),
  and a dev/production tracing subscriber.

### Phase 2: ORM and migrations v1
- `djangors-db`: sqlx-backed Postgres pool, config, transactions with explicit isolation levels.
- `djangors-orm`: `ModelMeta`/`FieldMeta`/`RelationMeta`, `#[derive(Model)]` (via
  `djangors-macros`), the `Expr` tree and `q!()` query macro, `QuerySet<T>` execution, model
  lifecycle (`save`/`update`/`delete`), aggregation (`Count`/`Sum`/`Avg`/`Min`/`Max`), `F()`
  expressions and race-safe bulk `update()`, and `select_related()` (a 2-query batched fetch, not
  a JOIN).
- `djangors-migrations`: a CreateTable-only v1 engine, wired to `dj migrate`.

### Phase 3: Templates, forms, static files
- `djangors-template`: MiniJinja-backed engine with loader precedence.
- `djangors-forms`: `FormField` trait, `CharField`/`IntegerField`/`BooleanField`/`EmailField`,
  `#[derive(Form)]`.
- `djangors-staticfiles`: dev serving, `collectstatic`, content-hashed manifest.

### Phase 4: Sessions, CSRF, and auth
- `djangors-sessions`: signed-cookie session engine.
- CSRF middleware, `ALLOWED_HOSTS`, HSTS, and `Secure` cookie flag support.
- `djangors-auth`: `User` model with Argon2id password hashing, `AuthBackend`, login/logout, the
  `Auth<U>` extractor, login rate limiting, and audit signals.
- `djangors-mail` (console backend) plus a password-reset flow.
- A fuzz/threat-model/OWASP security review pass, and `examples/polls` as the first real
  end-to-end app (login-gated voting).

### Phase 5: The admin site
- `djangors-admin`: `AdminSite` registry, `is_staff`-gated login, a real `createsuperuser`.
- Changelist: generic field rendering, sorting, pagination, `list_display`/`search_fields`,
  `list_filter`, bulk delete, `date_hierarchy`, `list_editable`, CSV export.
- Add/edit forms and delete confirmation (GET confirm + POST delete).
- A full permissions data model (`has_perm`) enforced across every admin view.
- Converted every admin page from hand-built HTML to the real template engine, added a proper
  HTML5 page shell with dark-mode CSS custom properties, per-site branding
  (`site_header`/`site_title`, favicons, logo/accent-color overrides), and CSRF-safe form wiring.
- Computed columns, bulk actions, fieldsets, readonly/`raw_id` fields, a transitive `PROTECT`
  walk on delete, a per-object history page, and extension points.
- A full audit log (`LogEntry`) with a "Recent actions" panel.
- `examples/school` as the Phase 5 Definition-of-Done app.

### Phase 6: Developer experience / CLI
- `dj new <name>` real project scaffolding, `dj new-app`, `dj run`, `dj check` (`--deploy`),
  `dj makemigrations` preview, `dj dbshell`, `dj test`, `dj shell`.
- `djangors-test`: a real `TestClient`/`TestDatabase`.
- Graceful shutdown for `Djangors::run`/`run_service`.

### Phase 7: Batteries
- `djangors-cache` (in-memory/database/Redis-backed `Cache` trait + `CacheLayer` middleware) and a
  complete `djangors-mail` (console/SMTP/file/in-memory backends).
- `djangors-contrib-messages`, a shared pagination utility, humanize template filters.
- `djangors-contrib-sitemaps`, `djangors-contrib-syndication` (RSS/Atom), `djangors-contrib-flatpages`,
  `djangors-contrib-redirects`.
- Real field-level audit diffing and `djangors-contrib-guardian` (object-level permissions).
- `djangors-i18n` (Fluent-based `.ftl` catalogs, `Accept-Language` parsing, locale-aware
  formatting) and `djangors-contrib-otp`.

### Phase 8: REST framework and background jobs
- `djangors-rest`: serializers, `ViewSet`s, router mounting, session/token/JWT auth and permission
  classes, filtering/ordering, and OpenAPI 3.1 generation.
- `djangors-core`: Server-Sent Events streaming with in-process broadcast groups.
- `djangors-tasks`: the `#[task]` macro, a `SKIP LOCKED`-based DB-backed queue, a worker loop, and
  admin integration.

### Phase 9: Documentation
- An 8-part polls tutorial mirroring Django's own.
- The mdBook docs site (`docs/src/`) with 8 topic guides, a Django-comparison guide, and 6
  how-tos.
- A real `evcxr`-backed `dj shell` REPL.
- 100% public-API rustdoc coverage across every crate (`#![deny(missing_docs)]` everywhere), and
  every documentation code block compiled as a real workspace test.
- The root `README.md`.

### Phase 10: Hardening, architecture parity, and 1.0 prep
- Honest `oha`-driven HTTP benchmarks against Django/Gunicorn and axum.
- Real migration autogeneration (new-table + new-field diffing against a schema snapshot) and a
  fix to a confirmed `dj migrate` bug found at the time.
- Eight architecture-parity items: compile-time-enforced scoped viewsets, `prefetch_related`
  batch eager-loading with N+1 regression tooling, a pluggable/project-customizable global error
  envelope, named+scoped rate limiting per endpoint, cron/scheduled recurring background jobs,
  cursor (keyset) pagination for REST viewsets, a pluggable `Storage` trait with
  `LocalDiskStorage`, and an `S3Storage` backend plus a real `FileField`.
- Load testing `djangors-admin` under concurrency, with a connection-pool tuning guide.
- A completed `djangors` facade crate, an API freeze review (both passes, all ~29 crates) with a
  published deprecation/stability policy.
- An internal security review (`cargo-audit` plus a manual audit, see
  `docs/security-review-2026-07-27.md`) that fixed an insecure-by-default cookie gap in both
  example apps. A real third-party audit is still open, pending a budget/vendor decision.
- The Astro marketing site (`site/`), deployed on Vercel, with AI-agent-friendly markdown
  (`llms.txt`/`llms-full.txt`, raw `.md` served at every doc URL, an on-page "Copy as Markdown"
  button) and a shipped Claude Code Skill (`.claude/skills/djangors-development/`).

### Phase 11: Django-parity gap closure
- CircleCI pipeline (fmt/clippy/build/test/doc-build against a real Postgres service container,
  plus a `cargo-audit` job). While validating it, found and fixed a real pre-existing
  test-isolation race in `djangors-tasks`'s test suite.
- Migration rollback with typed `Operation` variants (`AddColumn`/`DropColumn`/`AlterColumn`/
  `RenameColumn`) and real down-migrations.
- Model-level signals (`post_save`/`pre_save`/`post_delete`/`pre_delete`).
- `bulk_create` and a `ModelForm` equivalent auto-derived from `#[derive(Model)]`.
- Real `multipart/form-data` file upload parsing (not a stub).
- Server-rendered generic class-based views (`ListView`/`DetailView`/`CreateView`/`UpdateView`/
  `DeleteView`).
- A custom management-command plugin mechanism (`#[management_command]`, `dj <custom-command>`).
- A contenttypes/`GenericForeignKey` framework.
- `djangors-test`: transactional rollback-per-test and a JSON fixtures loader.
- A genuine cross-crate test DDL race (colliding global table names like `auth_user` across
  `djangors-admin`/`djangors-auth`'s own test suites) found and fixed via a real Postgres
  session-level advisory lock primitive (`djangors_test::acquire_cross_process_lock`), verified by
  re-reproducing the original collision as concurrent OS processes before and after the fix.
- First real production deployment (`djangors-polls` to Render), surfacing and fixing three real
  bugs along the way: unquoted reserved-keyword SQL identifiers in `derive(Model)`'s generated
  INSERT/UPDATE, a `dj makemigrations` bug where new models with foreign keys to sibling new
  models couldn't resolve the relation (planned one model at a time instead of together), and a
  Docker build missing TLS/`pkg-config`/`libssl-dev`.
- Most crates first published to crates.io as `0.1.0` (superseded by Phase 12 below; that
  snapshot predates every fix and feature listed there).
- **Not done**: the remaining two example-app deployments (only `djangors-polls` is live) and
  actually republishing the fixed/updated crates (Phase 12 does the fixing; the republish itself
  is still open, tracked in `PLAN.md`).

### Phase 12: Post-1.0 hardening
Compiled after the first real deployment surfaced genuine bugs, and after comparing Djangors
directly against a real production Django SaaS backend to find concrete gaps. Sequenced easiest
to hardest.
- Quoted every remaining raw SQL identifier in `QuerySet` (`.filter()`/`.order_by()`/
  `bulk_create`/`prefetch_related`/etc.), closing a gap the Phase 11 INSERT/UPDATE fix didn't
  cover.
- `#[derive(Settings)]`: a typed, validated app-config macro (the `pydantic-settings`/
  `django-environ` equivalent) reading `{PREFIX}_{FIELD}` env vars with defaults and
  `Option<T>` support.
- A `CspBuilder`/`CspLayer` middleware (the `django-csp` equivalent).
- Optional Sentry error tracking (`djangors-core`'s `sentry` feature) alongside the existing
  `tracing`-based logging.
- A `django-axes`-style persistent, database-backed account lockout (`PersistentLockoutBackend`),
  distinct from the existing in-memory rate limiter. It rejects even correct credentials once
  locked, and survives process restarts.
- `djangors-pdf`: typed PDF generation (report cards, invoices, receipts) via a pure-Rust builder
  API, deliberately not an HTML/Chrome-based renderer (no viable headless-Chrome path in a
  container deployment).
- Optional malware/AV scanning of uploaded bytes via `clamd`'s real `INSTREAM` wire protocol
  (`djangors-staticfiles`'s `clamav` feature), scanning in-memory before anything touches disk.
- `djangors-deploy`: a `DeployProvider` trait for `dj deploy`, shipping a `RenderProvider` (real
  REST API) and an `SshProvider` (shells out to the system `ssh`, avoiding a native SSH library
  dependency). A first slice, deliberately left to grow (Railway/GCP/AWS not yet implemented, no
  `dj deploy` CLI subcommand wired in yet).
- `djangors-contrib-payments`: a `PaymentProvider` trait and `PaystackProvider`, with an
  idempotency-key-first `Transaction` model (a real DB-level UNIQUE constraint on `reference`, not
  an application-level check-then-insert) and webhook handling that verifies the HMAC-SHA512
  signature against the raw body before ever parsing JSON. Real API shapes validated against a
  working production Paystack integration, not guessed.
- `djangors-contrib-tenancy`: multi-tenancy support, including a `Tenant` model, a
  `TenantMembership` join model, membership-verified per-request tenant resolution
  (`TenantResolutionLayer`, never trusts a client-supplied tenant header alone), and a one-line
  `tenant_scope()` helper reusing `djangors-rest`'s existing `Scoped`/`ScopedViewSet` enforcement
  mechanism. Design doc: `docs/design/12.1-multi-tenancy.md`.
- Workspace version bumped `0.1.0` to `0.2.0` and all 32 crates published for real, including
  `djangors-pdf`/`djangors-deploy`/`djangors-contrib-payments`/`djangors-contrib-tenancy` for the
  first time. Independently verified live on crates.io, not just attempted. Found and fixed a
  real bug along the way: 13 crates had internal *dev*-dependencies pinned with both `path` and
  `version` (a leftover from the version-bump script not distinguishing `[dependencies]` from
  `[dev-dependencies]`), which made publishing circular for crates with a mutual dev-dependency on
  each other. Fixed by keeping only `path` on internal dev-dependencies, which cargo correctly
  drops from the published manifest entirely.
- Also covered every Phase 12 feature in the mdBook doc site for the first time (new guides:
  settings, PDF, payments, multi-tenancy; extended auth/security/deployment guides). The doc site
  previously had zero mention of anything in this phase. Every code example genuinely compiles,
  verified via `doc-code-check`.

### Phase 12.1: dj new fix, 0.2.1 republish, and site polish
- Found and fixed a real bug in `dj new`: it located the sibling `djangors-core` crate via
  `env!("CARGO_MANIFEST_DIR")`, which only resolves correctly when `djangors-cli` is built from
  inside the cloned monorepo. Anyone who installed via `cargo install djangors-cli` from crates.io
  got a generated project with an unresolvable path dependency, and `cargo build` failed
  immediately. Fixed with a runtime check: use a path dependency when a sibling checkout exists,
  otherwise fall back to a real version dependency on the published crate.
- Workspace version bumped `0.2.0` to `0.2.1` and all 32 crates republished to ship the fix.
  Independently verified live on crates.io per crate, then re-verified end to end against the real
  registry (not the local `cargo package` simulation used during development): fresh
  `cargo install djangors-cli --version 0.2.1`, `dj new`, `cargo build`, and real HTTP requests to
  the generated project's `/` and `/healthz`, all green.
- mdBook docs pages gained the Copy-as-Markdown button (it previously only worked on the marketing
  site's own pages), and the button's contrast was fixed on both to meet WCAG AA (it was
  effectively invisible in dark mode due to an inherited low-contrast color).
- Marketing site SEO: real per-page meta descriptions (previously one generic line reused
  site-wide), Open Graph/Twitter Card tags, JSON-LD structured data, a sitemap, and `robots.txt`.
  The `site` canonical was also pointed at the framework's actual live URL, `djangors.vercel.app`
  (it had been set to `djangors.dev`, a domain that was never registered).
- Homepage gained four new content sections (pillars, get-started, features grid, a real 4-column
  footer with every link verified to resolve), and an em-dash reduction pass across the site copy,
  README, and every doc guide.
- Corrected an earlier (Phase 11) conclusion that had closed `tick_recurring_tasks`'s dual-claim
  race as "not a real bug, misattributed to cross-crate test load." It really is a genuine,
  deterministic bug: whenever a concurrent tick runs within roughly the first 60 seconds after a
  cron boundary, a single-step schedule advance can land on an occurrence that's still due,
  wrongly making the earlier fix's 520 clean verification runs a matter of wall-clock luck rather
  than proof. Captured the tick's `now` after acquiring its advisory lock instead of before, and
  fixed the regression test's setup to seed a realistic, boundary-aligned overdue value instead of
  an arbitrary offset. Deliberately preserved the intentional one-occurrence-per-tick catch-up
  design (multiple concurrent ticks are meant to share backlog work, not each grab everything at
  once).

### Phase 12.2: Error handling, ORM transactions, and REST polish

Driven by an external review of a real backend built on 0.2.2, which rated error handling 4.5/10
and REST 5.5/10 as the framework's weakest areas.

**Error handling.** `DjangorsError` was a closed 7-variant enum whose payload was a bare `String`,
with `code()` and `message()` private — so an application could not attach its own status code,
stable domain code, or structured details, and even a custom `ErrorRenderer` could not read them
back out.
- Added `DjangorsError::Api(ApiError)`, carrying an explicit `StatusCode`, a stable domain `code`,
  a `message`, and an optional `serde_json::Value` of `details`. Constructed via
  `DjangorsError::api(status, code, message)` and `.with_details(json)`; `.with_details()` on a
  built-in variant promotes it, preserving status/code/message.
- Made `code()` and `message()` public and added `details()`, so custom renderers can build their
  own envelope without re-matching on the variant.
- Added the `ApiResultExt` trait (`.api_err(status, code)` / `.api_err_msg(...)`) so foreign errors
  convert at the `?` site without a `map_err` closure.
- Error responses now content-negotiate: a project `ErrorRenderer` wins, then the JSON envelope
  when the caller sent `Accept: application/json` or the error is an `ApiError`, then the debug or
  production page. This applies to every route, not just REST. The six duplicated
  render-the-error branches in `router.rs` collapsed onto one `DjangorsError::render` method.
- The JSON envelope now omits `details` entirely when absent rather than emitting `null`.

**ORM transactions.** `Database::transaction` existed, but `QuerySet` only ever executed against
`Database::pool()`, so no ORM call could join a transaction — atomic work had to drop to raw sqlx
and give up the `QuerySet` API.
- Added `djangors_db::DbExecutor`, implemented for `&Database` (the pool) and `&mut PgConnection`
  (an open transaction, as handed to the `transaction` closure), plus the `Conn` handle that
  centralises the pool-versus-transaction dispatch.
- Every `QuerySet` method is now generic over `DbExecutor`: `all`, `get`, `first`, `exists`,
  `count`, `aggregate`, `update`, `insert_raw`, `bulk_create`, `delete_by_pk`, `select_related`,
  and the free `prefetch_related`. Existing `&Database` call sites are unchanged.
- Added `impl From<OrmError> for DbError` (and a `DbError::Orm` variant). Without it the ORM still
  could not be used inside `transaction`, whose closure must return an error convertible to
  `DbError` — the trait bound, not the executor, was the actual blocker.
- **Breaking:** `select_related` and `prefetch_related` gained a trailing inferred type parameter,
  so turbofished calls become `select_related::<Related, _>(..)` and
  `prefetch_related::<Parent, Child, _>(..)`.

**REST.**
- Page size is configurable per endpoint via `ViewSetConfig::page_size`; it was a hardcoded
  `REST_PER_PAGE = 100`. Setting `max_page_size` additionally opts the endpoint into a
  client-supplied `?page_size=`, clamped to that cap. Unparseable or out-of-range values fall back
  to the default rather than failing an otherwise valid request.
- Added the `IsStaff`, `IsSuperuser`, and `IsReadOnly` permission policies, the `And`/`Or`/`Not`
  combinators, and the `PermissionExt` trait (`.and()`, `.or()`, `.negate()`) — previously the only
  policies were `AllowAny` and `IsAuthenticated`, with no way to compose them.
- Added `current_user()`, which resolves the request's user across session, token, and JWT auth
  using the same precedence as `IsAuthenticated`.

**REST, part two — the serializer layer.** `djangors-rest` was a single 3,257-line `lib.rs` whose
`serialize`/`deserialize` were direct `Model`-to-JSON with no field control and no validation hook.
- Split into `auth`, `permissions`, `serializers`, `validation`, `pagination`, `viewsets`, and
  `openapi` modules. Every name is re-exported from the crate root, so existing imports are
  unaffected.
- Added the `Serializer<M>` trait (`to_representation` / `to_internal_value` / `validate` / `parse`)
  and `ModelSerializer<M>`, the metadata-driven default. `ModelSerializer` with `FieldSet::all()`
  reproduces the previous behaviour exactly.
- Added `FieldSet`: `only` / `excluding` / `read_only` / `write_only`, giving the read/write split
  the review found missing. A write to a read-only field is now *rejected* rather than silently
  dropped — a client that thinks it set `id` is told it did not.
- Added `ValidationErrors`: a `{field: [messages]}` map with `non_field_errors`, which renders as a
  `422` `DjangorsError::Api` carrying the whole map in `details`. Validation failures used to
  collapse into one `BadRequest` string, so a client could not tell which field was wrong.
- Added the `Validator<T>` trait and `ModelSerializer::with_validator` for object-level rules. Every
  registered validator runs even if an earlier one failed, so the client sees the full set at once.
- Added the `Pagination` trait with `PageNumberPagination` (the default), `LimitOffsetPagination`,
  and `CursorPagination`. The strategy owns both the row window and the response envelope.
- Added `ViewSetOptions<M>` (serializer + pagination + config), `*_with_options` handlers for
  `list`/`retrieve`/`create`/`update`, and `viewset_routes_with_options` so the options reach every
  handler rather than just `list`.

**Two real bugs found while building the above, both caught by new tests.**
- `PATCH` was not partial. `deserialize` walks every column and defaults the absent ones, so a
  `PATCH` of one field would persist a `false` for every omitted boolean and `NULL` for every
  omitted nullable — silently resetting columns the client never mentioned. `deserialize` is now
  partial-aware (`deserialize_partial`), and absent keys are skipped rather than defaulted.
- A missing non-nullable boolean was silently accepted as `false` on a full write. That is right for
  an HTML checkbox and wrong for a JSON API, where `POST {}` would quietly clear a flag.
  `ModelSerializer` now reports it as required; `deserialize` keeps the form semantics the admin
  depends on.

**Router state and middleware ergonomics.**
- Added `Request::require_state<T>()`, returning a descriptive error naming the missing type instead
  of the `.ok_or_else(|| ...Internal("Database connection not found"))?` that appeared 30 times
  across the framework. All 30 call sites now use it.
- `Router::mount` now inherits state from the sub-router for types the parent has not set (the
  parent still wins on conflict). Previously a mounted sub-router's state was dropped, so a handler
  that worked standalone would fail with "state absent" once mounted.
- Added `AppState::merge`, `contains`, `len`, and `is_empty`.


### Phase 12.3: ORM query expressiveness, REST filtering/throttling, and a responsive site

Continues the response to the 0.2.2 external review. Phase 12.2 addressed error handling and the
REST serializer layer; this phase addresses the remaining low scores — the ORM's query surface
(rated 6/10) and the rest of the DRF parity gap — plus the marketing site's mobile layout.

**ORM: `Q`-object filters.** `Expr` already supported `And`/`Or`/`Not` internally and the SQL
compiler already emitted all three, but `UnresolvedExpr` — the type `q!` produces — had a single
`And` variant, so nothing a user could write ever reached the `Or` or `Not` paths. Two ad-hoc
escape hatches (`filter_or_icontains`, `filter_datetime_range`) existed precisely because of this.
- `UnresolvedExpr` is now a tree: `All`, `Any`, and `Negate` join the original `And` leaf, with
  `BitAnd`/`BitOr`/`Not` operator impls. `q!(a = 1) | q!(b = 2)` is Django's `Q(a=1) | Q(b=2)`.
- `QuerySet::filter` resolves the tree recursively, validating every field name at every depth.
- Added `QuerySet::exclude`, Django's `.exclude()`.
- The variants are additive, so the ~15 existing `UnresolvedExpr::And(...)` construction sites
  across the workspace are unchanged.

**ORM: lookup suffixes.** Added `__ne`, `__iexact`, `__in`, `__isnull`, `__regex`, and `__iregex`
to the existing comparison and substring lookups.
- `__in` takes a `Vec` via the new `Value::List`, expanded into one placeholder per element while
  compiling. An empty list compiles to `FALSE` rather than the syntactically invalid `IN ()`,
  matching Django's `__in=[]`.
- `__isnull` binds no parameter at all, and `= false` inverts it.
- Both `suffix_to_op` and `split_field_lookup` were updated; missing the latter is what made the
  first round of lookup tests fail, since an unrecognised suffix is treated as part of the field
  name and surfaces as `FieldNotFound`.

**ORM: `F` expressions in filters.** `F` previously existed only for the update side
(`set!(votes = F("votes") + 1)`). Added `Expr::CompareField` and the `q_f!` macro for
column-to-column comparison in a `WHERE` clause (`q_f!(paid_at__gte due_at)`), which binds no
parameters.

**ORM: grouping and projections.**
- `QuerySet::annotate(db, group_by, aggs)` — Django's `.values(...).annotate(...)`. Returns
  `GroupRow`s carrying the group keys and named aggregate results. Ordering is dropped for grouped
  queries, since ordering by a column outside the `GROUP BY` is not valid SQL.
- `QuerySet::values` / `values_list` for column projections.
- `QuerySet::debug_sql` / `debug_params` — Django's `str(queryset.query)`. Placeholders stay as
  `$1`, `$2`, … rather than being interpolated, so the output is never a runnable statement.
- Added `OrmError::InvalidQuery` for queryset misuse caught before reaching the database.

**REST: filter backends.** `ViewSetConfig::filterable_fields` only ever produced exact matches.
Added a `FilterBackend<M>` trait with three implementations, composed in order via
`ViewSetOptions::with_filter_backend`:
- `FieldFilter` — lookup suffixes (`?age__gte=18`, `?status__in=a,b`, `?deleted_at__isnull=true`).
- `SearchFilter` — free-text `?search=` across configured fields.
- `OrderingFilter` — allowlisted `?ordering=-field`.

All three are allowlist-driven, and the field/lookup name handed to the ORM is rebuilt from the
compile-time allowlist rather than taken from the query string, so a client-supplied name cannot
reach SQL.

**REST: throttling.** Added `Throttle` and `parse_rate`, built on the existing
`djangors_core::ratelimit` sliding window rather than duplicating it. DRF-style rate strings
(`"100/hour"`), keyed per authenticated user and falling back to client IP (`ByUserOrIp`), applied
to every ViewSet action via `ViewSetOptions::with_throttle`. A malformed rate returns `None`
instead of silently becoming a default budget.

**REST: nested serializers.** Added `NestedSerializer` and
`Serializer::to_representation_nested`, which embed a pre-fetched related object in place of its
raw foreign key. The serializer trait is synchronous and issues no queries, so this pairs with
`select_related`; without a loaded relation the field keeps its id rather than becoming `null`.

**Ergonomics.** `Value` now converts from `i8`/`i16`/`i32`/`u8`/`u16`/`u32`/`f32`, not just
`i64`/`f64`. A model field declared `i32` — the common case for Django's `IntegerField` — could
not previously be used with `q!` without a manual cast. This surfaced from the compile-tested doc
examples, which the `polls` example's `votes: i32` field exercises.

**Bug fixed: `contains` on a non-text value built a `Debug`-formatted pattern.** The `LIKE`/`ILIKE`
branches formatted non-`Text` values with `{:?}`, so `q!(field__contains = 5i64)` bound
`%I64(5)%` instead of `%5%` and matched nothing. All four affected branches now use `Display`.

**Docs.**
- New topic guide: **REST Framework** (`docs/src/guides/rest.md`), covering ViewSets, serializers,
  validation, nested serializers, all three pagination strategies, filter backends, permissions,
  throttling, and the JSON error envelope. There was previously no REST guide at all, despite REST
  being the review's second-lowest-rated area.
- The ORM guide gains a lookup-suffix table and sections on combining filters with `OR`/`AND`/
  `NOT`, `q_f!`, and `annotate`/`values`. Every example compiles under `tools/doc-code-check`.

**Site: mobile.**
- The viewport meta was `width=device-width` with no `initial-scale=1`.
- Pinned the Vite CSS target. Without one the minifier rewrote every `@media (max-width: …)` into
  Media Queries Level 4 range syntax (`@media (width <= …)`), which iOS Safari only understands
  from 16.4 — on an older iPhone *no* breakpoint matched and the desktop layout was served, which
  defeats the point of the responsive rules. This also affected the pre-existing breakpoints.
- Added breakpoints at 900/650/480px plus a `pointer: coarse` block: the hero and headline scale
  down, every grid reaches a single column, long URLs and code no longer force horizontal page
  scroll, the "Copy as Markdown" button no longer overlaps the first heading, and tap targets meet
  the 44px guideline.

### Phase 13.1: SQLite backend (dual-backend foundation)

Djangors was PostgreSQL-only. This adds SQLite as a second backend for the ORM query path,
the derive macro, and the migration type mapping — enough for `dj new` → running app with no
database server, and for a test suite that needs no Postgres.

`sqlx::Any` was evaluated and rejected: its `AnyValueKind` is
`Null|Bool|SmallInt|Integer|BigInt|Real|Double|Text|Blob` with **no chrono support**, and every
model in this workspace uses `DateTime<Utc>`. Making `QuerySet`/`FromRow`/the derive macro generic
over `sqlx::Database` was also rejected — it would propagate `for<'r> i64: Decode<'r, DB> + Type<DB>`
bounds through every method and every downstream crate. The chosen design is enum dispatch confined
to `djangors-db`.

**New in `djangors-db`:**
- `Dialect` (`Postgres` | `Sqlite`) — placeholder style (`$1` vs `?`), `ILIKE` vs `LIKE`, identifier
  quoting, float casts.
- `BindValue` / `NullKind` — a driver-independent parameter. `NullKind` carries the typed-NULL
  information that `NullBindKind` used to provide inside the ORM, because Postgres rejects a
  mismatched parameter type even for NULL.
- `DbRow` — wraps `PgRow`/`SqliteRow` with typed accessors by index and by name.
- `Conn` is now four-way (`PgPool`/`PgTx`/`SqlitePool`/`SqliteTx`) and its methods take
  `(sql, &[BindValue])` rather than a pre-built `sqlx::Query`, which is driver-typed and cannot
  cross a backend boundary.
- `Database` is an enum over the two pools; `connect()` picks the backend from the URL scheme.
  `pool()` still returns `&PgPool` and **panics on a SQLite handle**, so not-yet-ported raw-SQL
  call sites fail loudly rather than silently misbehaving. `sqlite_pool()` returns `Option<&SqlitePool>`.

**Simplification:** because `Conn` now binds parameters itself, the **eight duplicated bind blocks
in `queryset.rs` and four more in the derive macro are gone** — twelve copies of the same
`match value { … .bind(…) }` collapsed into two (one per driver).

**Breaking:** `FromRow::from_row` takes `&DbRow` instead of `&sqlx::postgres::PgRow`. This is a
public trait implemented by every `#[derive(Model)]` type.

**Integer decoding** now falls back `i64 → i32 → i16`, widening losslessly. Postgres's binary
protocol is width-strict: an `INT4` column cannot be decoded as `i64`. SQLite is dynamically typed
and hides this entirely, so a SQLite-only test would never have caught it — eight admin tests did.

**SQLite type mapping** (`sql_type_for` is dialect-aware): `VARCHAR(n)`/`TEXT`→`TEXT`,
`DOUBLE PRECISION`→`REAL`, `BOOLEAN`→`INTEGER`, date/time/`INTERVAL`/`UUID`/`INET`/`JSONB`→`TEXT`,
`BYTEA`→`BLOB`. An auto primary key becomes `INTEGER PRIMARY KEY AUTOINCREMENT`, and `column_sql`
suppresses its usual `NOT NULL`/`PRIMARY KEY` for such a column — SQLite rejects a table with two
primary keys.

**Known limitations (deliberately deferred):**
- The ~555 raw `sqlx::query` call sites in `djangors-admin`, `-auth`, `-rest`, `-tasks`, and
  `djangors-contrib-*` still write `$N` placeholders and are **Postgres-only**.
- `dj makemigrations` and `plan.rs` hardcode `Dialect::Postgres`, so generated migration files are
  still Postgres DDL.
- Therefore SQLite currently supports the ORM query path, not a whole running admin site.

400 tests pass.

### Phase 13.2: Dialect-aware raw SQL

13.1 made the ORM dual-backend, but every crate that drops to raw SQL still wrote `$N`
placeholders and called `Database::pool()`, which panics on a SQLite handle — so an app on SQLite
got a working ORM and an admin that panicked on the first request. This closes that.

**Scope correction.** 13.1's notes said "~555 raw `sqlx::query` sites". That count included
test-setup SQL. The *production* surface was **31**: admin 4, auth 3, tasks 11, guardian 5,
contenttypes 4, cache 4. All are now dialect-aware.

**Two cases were not mechanical:**

- **`EXTRACT` (admin date_hierarchy).** SQLite has no `EXTRACT`. Added
  `Dialect::extract_date_part(DatePart, col)`: `EXTRACT(YEAR FROM col)::int` on Postgres,
  `CAST(strftime('%Y', col) AS INTEGER)` on SQLite. The `CAST` is load-bearing — `strftime`
  returns a zero-padded *string*, so without it the comparison against a bound integer silently
  matches nothing.
- **`pg_advisory_xact_lock` (tasks).** SQLite has no advisory locks and does not need one here: it
  permits a single writer and a write transaction holds an exclusive database lock for its
  duration, so the interleaving the advisory lock defends against cannot occur. The lock is now
  issued only under `Dialect::Postgres`, with that reasoning recorded at the call site rather than
  left as a silent omission.

Also added `Dialect::bytea_type()` (`BYTEA`/`BLOB`) for djangors-cache's DDL.

**Still Postgres-only (deferred):** `dj makemigrations` and migration *plan* generation, and the
`djangors-test` harness (whose job is provisioning isolated Postgres databases). Neither is an
application-serving path. *(makemigrations and plan generation were closed in Phase 13.3 below.)*

**Review finding.** The dispatch added a cross-process lock to
`examples/polls/tests/voting.rs` after seeing that test fail with `401 "user not found"`. That
failure was an artifact of two `cargo test` runs executing concurrently against the same database
(the auth suite drops and recreates `auth_user` mid-flight). Verified by running the suite with the
change reverted: 401 tests pass. Reverted, since it would otherwise have shipped two
`std::mem::forget` leaks and an unneeded dev-dependency into an example app that users read as
reference code.

401 tests pass.

### Phase 13.3: Dialect-aware migrations (`dj makemigrations`, `dj migrate`, plan/DDL)

13.1 made the ORM dual-backend and 13.2 did the same for production raw SQL, but the piece that
*creates* the schema was still Postgres-only — a SQLite project could run its ORM and admin only
against tables it had hand-written. This closes the `dj new` → SQLite → running admin story.

**The blocker.** `Database::transaction` takes `FnOnce(&mut PgConnection)` and returned
`TransactionFailed` on a SQLite handle. Every migration apply/rollback path runs inside one, so
nothing downstream could be ported until it was solved. `Conn` already had `PgTx`/`SqliteTx`
variants with no `Database` method handing one out, so this adds `Database::transaction_conn`
(dual-backend, commit on `Ok`, rollback on `Err`) *alongside* the existing methods, which are
unchanged.

**New `Dialect` helpers:** `auto_pk_type()` (`SERIAL PRIMARY KEY` / `INTEGER PRIMARY KEY
AUTOINCREMENT`), `timestamp_type()` (`TIMESTAMPTZ` / `TEXT`), `current_timestamp()` (`now()` /
`CURRENT_TIMESTAMP`), and `from_url()`. `timestamp_type` is `TEXT` on SQLite, not `TIMESTAMP`:
sqlx's SQLite `DateTime<Utc>` codec reads and writes ISO-8601 *text*, and `TEXT` affinity stores
that losslessly where `TIMESTAMP`'s NUMERIC affinity would coerce. `Database::connect` now
delegates to `from_url` rather than keeping a second copy of the URL detection.

**Plan and DDL.** `build_create_all_plan` and `build_create_plan_from_snapshots` take a `Dialect`
instead of hardcoding Postgres. `Operation::to_sql` is now dialect-aware **and fallible**:
`AlterColumnType` has no SQLite equivalent (SQLite's `ALTER TABLE` cannot change a column's type;
the real workaround is a 12-step table rebuild), so it returns the new
`MigrationError::UnsupportedOnDialect` rather than emitting Postgres syntax that would fail at
apply time. A generated migration file that is silently wrong is worse than one that refuses to
generate. The plan builders never construct that variant, so no working path changed.

**Runner.** All `Database::pool()` calls in `djangors-migrations` and the CLI's `migrate_with_plan`
now go through `Conn`; the history-table DDL is built from `Dialect` instead of three copies of a
Postgres literal. `rollback_from_dir`'s `WHERE name = ANY($1)` (no SQLite array binding) became an
expanded `IN (?, ?, …)` with an empty-input guard, since `IN ()` is a syntax error in both
dialects. `makemigrations` is synchronous and never connects, so it infers its dialect from
`DATABASE_URL` via `Dialect::from_url`, defaulting to Postgres when unset.

**Still Postgres-only (deferred):** the `djangors-test` harness, whose job is provisioning isolated
Postgres databases and whose isolation relies on Postgres session-level advisory locks.

405 tests pass (401 + 4 new).

### Phase 13.4: Documentation pass

Measured doc-line-to-code-line ratio across every crate and documented the ten thinnest.
`djangors-admin` was both the largest crate in the workspace and the least documented (9074 code
lines, 1.5%). Added explanatory comments — the "why", not restatements of the "what" — with
priority on module headers, private helpers, and non-obvious logic. Several rationales that
previously existed only in `CHANGELOG.md` or `docs/design/*.md` were brought down to the code they
describe (the `Vec<(String,String)>`-vs-`HashMap` form-extractor decision, the `html_escape`
`&#x2F;` vs minijinja `&#x2f;` escaping discrepancy, why `Conn` uses enum dispatch rather than
generics over `sqlx::Database`).

Constrained to comments only: verified by stripping all comments from both revisions of every
touched `.rs` file and confirming the remainder was byte-identical. Zero code changed in 13 files.

Eight new topic guides for subsystems that had no prose documentation: `databases.md` (the
13.1–13.3 dual-backend story, previously undocumented anywhere), `migrations.md`, `tasks.md`,
`caching.md`, `i18n.md`, `sessions.md`, `static-files.md`, `signals.md`.

**Known limitation:** every Rust block in the new guides is tagged `rust,illustrative`, so none is
compile-checked by `doc-code-check`. API names were instead verified by hand against real source.
Promoting the runnable ones to `rust,compile` is follow-up work.

405 tests pass.

## [0.6.0]

### Phase 14.1: Dual-backend test harness

The original reason for adding SQLite was that tests would run faster. 13.1–13.3 made *application*
code dual-backend but left the framework's own suite Postgres-only — ~475 test SQL sites plus 11
Postgres-specific mechanisms in `djangors-test`. This closes that.

`djangors-test` gains `TestBackend { Postgres, Sqlite }` and `TestDatabase::new_for_backend()`,
selected by `TEST_BACKEND` or by whether `DATABASE_URL` is set. SQLite needs neither an advisory
lock nor `CREATE DATABASE`: a fresh `sqlite::memory:` handle **is** a private database, so
per-test isolation is free and strictly stronger than the Postgres scheme.

The SQLite pool is pinned to `max_connections(1)`. With a plain `sqlite::memory:` URL every pooled
connection is a *separate* database, so setup DDL executed on one connection would be invisible to
the next query on another.

Ported: `djangors-test`, `-db`, `-migrations`, `-contrib-guardian`, `-views`, `-tasks`, `-auth`,
`-orm`, `-rest`, `-admin`.

**This is dual-mode, not a switch.** Postgres stays first-class — isolation levels,
`pg_advisory_lock`, `SKIP LOCKED`, and width-strict INT4/INT8 decoding are only meaningfully
covered there.

Measured on a development machine: `djangors-admin`'s 32 tests run in **0.69s** on SQLite versus
**15.8s** on Postgres; `djangors-rest`'s 45 in **0.44s** versus **4.7s**.

**Caveat, stated plainly:** five tests early-return when `DATABASE_URL` is unset, each with a
comment naming the Postgres feature required. They report as *passing* on SQLite without
executing, so a green SQLite run alone does not prove those paths still work. Run against Postgres
before changing anything dialect-specific.

Backend selection and this caveat are documented in `docs/src/guides/testing.md`.
