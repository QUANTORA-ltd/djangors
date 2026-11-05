<p align="center">
  <img src="assets/logo.svg" alt="Djangors logo" width="220">
</p>

<h1 align="center">Djangors</h1>

<p align="center">
  <b>The Django of Rust.</b><br>
  Everything that makes Django pleasant: the ORM, migrations, the admin, forms, auth, the
  batteries, the docs, the tutorial. All of it with Rust's speed, safety, and single-binary deploys.
</p>

<p align="center">
  <a href="https://github.com/QUANTORA-ltd/djangors/blob/main/PLAN.md">Roadmap</a> ·
  <a href="docs/src/tutorial/01-requests-and-responses.md">Tutorial</a> ·
  <a href="docs/src/django-comparison.md">Djangors for Django developers</a> ·
  <a href="docs/src/guides/">Topic guides</a>
</p>

---

## What is this?

Django made building web applications feel obvious: models describe your data, the admin gives you
a working back office for free, `manage.py` handles the busywork, and the framework has an answer
for everything from sessions to CSRF to internationalization. Djangors brings that exact experience
to Rust. It's batteries included, not a thin HTTP toolkit you have to assemble yourself, and it also
gets the wins Rust is actually good at: no null-pointer/type-confusion bugs reaching production, no
GC pauses, and a `cargo build --release` that produces one static binary you can just copy to a
server and run.

It's built for the workloads Django gets nervous about at scale: banking backends, school
management systems, high-traffic e-commerce. The kind of system where "it usually works" isn't
good enough.

**Status**: pre-1.0, under active development. The core (ORM, admin, auth, migrations, CLI,
contrib batteries, a REST framework, real-time/SSE, background tasks, payments, multi-tenancy) is
built and tested; see [`PLAN.md`](PLAN.md) for the full phase-by-phase roadmap and what's done vs.
remaining. All 32 crates are published on
[crates.io](https://crates.io/search?q=djangors) at `0.6.2`, including the full contrib battery:
PDF generation, deployment, payments, multi-tenancy, sitemaps, RSS/Atom syndication, flat pages,
redirects, flash messages, object-level permissions, TOTP/2FA, and content types / generic FKs.

## Why Rust instead of Python for this?

- **Compile-time correctness.** A typo'd field name, a wrong type passed to a query, a missing
  `Result` handled: Django (and Python generally) finds these at runtime, sometimes in production.
  `rustc` finds them before the code ships.
- **No GIL, no GC pauses.** Djangors runs on Tokio's async runtime: real concurrency, not
  cooperative multitasking behind a global lock.
- **One binary, not a Python environment.** `cargo build --release` produces a single static
  executable with everything baked in. No virtualenv, no "works on my machine" dependency drift,
  no separate WSGI/ASGI server + static-file server dance to wire up for production.
- **Memory safety by construction.** No use-after-free, no data races, enforced by the compiler
  instead of by discipline.

## What's included

Djangors is a Cargo workspace of focused crates, mirroring how Django itself is one framework made
of composable pieces:

| Crate | What it does |
|---|---|
| `djangors-core` | The HTTP kernel: `Request`/`Response`, the `Router`, middleware (CSRF, security headers, HSTS, CSP builder, host validation), sessions-adjacent state, SSE streaming, signals (framework + model lifecycle), real `multipart/form-data` file upload parsing, graceful shutdown, optional Sentry error tracking |
| `djangors-orm` | The ORM: `QuerySet`, filter/order/aggregate expressions, model metadata, `bulk_create`, idempotent `get_or_create`/`update_or_create`, Postgres full-text search, `EXPLAIN`, and scalar DB function expressions (`COALESCE`/`LOWER`/`UPPER`/`CONCAT`/`LENGTH`) |
| `djangors-macros` | `#[derive(Model)]` (also generates a `ModelForm` equivalent), `#[derive(Settings)]` (typed, validated app config), `#[task]`, and `#[management_command]` attribute macros; `auto_now`/`auto_now_add` timestamps, field `choices` (with DB CHECK constraints), and UUID/Date/Decimal support |
| `djangors-db` | Connection pooling and config-driven database setup (Postgres) |
| `djangors-migrations` | Schema migrations with real per-file history and rollback (`dj migrate --rollback`) |
| `djangors-auth` | Users, groups, permissions, session-backed auth, password hashing, rate-limited login, persistent account lockout |
| `djangors-sessions` | Signed-cookie session engine |
| `djangors-admin` | The auto-generated admin site: changelist, filters, search, bulk actions, inline editing, CSV export, audit log, object history, FK inlines, and optional tenant scoping |
| `djangors-forms` | Form field types and validation, plus the `ModelForm` equivalent auto-derived from `#[derive(Model)]` |
| `djangors-views` | Server-rendered generic class-based views: `ListView`/`DetailView`/`CreateView`/`UpdateView`/`DeleteView` |
| `djangors-template` | A Django-template-flavored engine (minijinja-backed) with Django-style filters |
| `djangors-rest` | A DRF-equivalent: generic serialization, `ViewSet`s (scoped with config via `scoped_viewset_routes_with_config`), token/JWT auth, permission classes, cursor pagination, rate limiting, OpenAPI 3.1 generation, and a `request.user()` convenience accessor |
| `djangors-cache` | Cache trait + in-memory/database/Redis backends |
| `djangors-mail` | Email messages with SMTP/file/in-memory backends |
| `djangors-tasks` | A background task queue (`#[task]`, Postgres `SELECT ... FOR UPDATE SKIP LOCKED`, cron-style recurring jobs, a worker loop) |
| `djangors-pdf` | Typed PDF generation for report cards, invoices, and receipts: headings, text, and tables with automatic page breaks |
| `djangors-i18n` | Runtime internationalization (Fluent-backed catalogs, locale fallback chain) |
| `djangors-staticfiles` | Static file collection and serving, a pluggable `Storage` trait (local disk, S3), optional malware/AV scanning of uploaded bytes via `clamd` |
| `djangors-test` | In-process test client, real per-test database isolation, and a JSON fixtures loader |
| `djangors-cli` (`dj`) | The `manage.py` equivalent: `new`, `run`, `migrate` (with rollback), `createsuperuser`, `shell` (a real `evcxr` Rust REPL), `dbshell`, `test`, `check --deploy`, and a plugin mechanism for a project's own `dj <custom-command>` |
| `djangors-deploy` | `DeployProvider` trait for `dj deploy`. Ships a Render provider (real REST API) and an SSH/VPS provider (shells out to the system `ssh`); Railway/GCP/AWS not yet implemented |
| `djangors-contrib-payments` | Payment provider integration (Paystack) with idempotent, DB-constraint-backed transaction recording; amounts always in integer minor units, never a float |
| `djangors-contrib-tenancy` | Multi-tenancy: a `Tenant` model, membership-verified per-request tenant resolution, and a one-line `Scoped` helper for real cross-tenant row isolation |
| `djangors-contrib-*` | Sitemaps, RSS/Atom syndication, flat pages, redirects, flash messages, object-level permissions (guardian-style), TOTP/2FA, content types / generic foreign keys |

Two full example apps exercise the framework end-to-end: `examples/polls` (the tutorial app, mirrors
Django's own polls tutorial) and `examples/school` (an admin-heavy example proving the generic admin
alone is enough to build a real CRUD app).

## Quick start

Install Rust if you don't already have it, then the `dj` CLI from crates.io:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # macOS/Linux; on Windows, winget install Rustlang.Rustup
cargo install djangors-cli
```

Scaffold a new project and run it:

```bash
dj new mysite
cd mysite
DATABASE_URL="postgres://postgres:postgres@localhost/mysite_dev" cargo run
```

Then follow the real, verified [8-part tutorial](docs/src/tutorial/01-requests-and-responses.md). It
builds the same polls app Django's own tutorial does, one part at a time, using this exact
codebase's real APIs (every snippet in it is compiled and checked in the workspace's own test
suite, see `tools/doc-code-check`).

When you move beyond the tutorial, read the [Application Architecture guide](docs/src/guides/application-architecture.md).
It explains the recommended beginner-friendly boundary between models, contracts, repositories,
services, views, URLs, permissions, and tests, with examples of how a request flows through them.
Rust web developers can also use [Moving from Actix Web or Axum](docs/src/migration/from-actix-axum.md),
which maps handlers, extractors, state, middleware, errors, databases, tasks, and testing to Djangors.

Coming from Django? Start with
[**Djangors for Django developers**](docs/src/django-comparison.md), a direct, side-by-side
translation reference.

### Building from source

Contributing to Djangors itself, or want a `dj` build with unreleased fixes? Clone the monorepo
instead of installing from crates.io:

```bash
git clone https://github.com/QUANTORA-ltd/djangors.git
cd djangors
cargo build --workspace
cargo run -p djangors-cli -- new mysite
```

## Logging

Djangors uses the standard Rust [`tracing`](https://docs.rs/tracing) ecosystem, with two ready-made
entry points in `djangors-core::logging`:

- **`init_dev_logging()`**: compact, colored console output; call it first thing in `main()` during
  development. Respects the standard `RUST_LOG` environment variable (e.g. `RUST_LOG=debug` or
  `RUST_LOG=djangors_core=trace`), defaulting to `info,djangors_core=debug`.
- **`init_production_logging()`**: structured JSON output instead of colored text, ready to pipe
  into a log aggregator (Elasticsearch, CloudWatch, Datadog). Same `RUST_LOG` support, defaulting to
  `info,djangors_core=info`.

Since both are built on `tracing`, any `tracing`-instrumented code, including this framework's own
middleware, shows up automatically once one of them is initialized. It's the closest Djangors
equivalent to Django's default `runserver` request-logging line.

```rust
djangors_core::logging::init_dev_logging();
```

## Documentation

The full docs site (tutorial, topic guides, the Django-comparison guide, how-tos) lives under
[`docs/src/`](docs/src/) and builds with [mdBook](https://rust-lang.github.io/mdBook/):

```bash
cargo install mdbook
mdbook build docs
mdbook serve docs   # live-reloading local preview
```

Every Rust code block in the docs is compiled as part of `cargo test --workspace` (see
`tools/doc-code-check`), so the examples you read are guaranteed to actually work against this
exact codebase, not just look plausible.

## Roadmap

See [`PLAN.md`](PLAN.md) for the full 10-phase plan, what's done, and what's left before 1.0.

## License

Dual-licensed under `MIT OR Apache-2.0`. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE) for the full license texts.
