# Djangors — The Django of Rust

**One-line pitch:** Everything that makes Django pleasant — the ORM, migrations, the admin, forms, auth, the batteries, the docs, the tutorial — with Rust's speed, safety, and single-binary deploys. Built for the workloads Django gets nervous about: banking backends, school management systems, high-traffic e-commerce.

**North star test:** A developer who knows Django should be able to read the Djangors tutorial and feel *at home* within an hour. A CTO at a bank should be able to answer "why Djangors?" with: memory safety, no runtime type errors in production, 10–50x throughput per server, and a compile step that catches what Django catches only in production.

---

## Part 0 — Non-negotiable principles

These decide every argument later. Write them down now, never violate them.

1. **Batteries included.** If Django ships it, Djangors ships it. No "just wire up these 9 crates yourself."
2. **Convention over configuration, configuration always possible.** Sensible defaults everywhere; every default overridable.
3. **The admin is a first-class product, not a demo.** It's the killer feature. It must be good enough that a school secretary or bank ops person uses it daily.
4. **Compile-time where it helps, runtime where Django's flexibility matters.** Models and queries: checked at compile time. Templates and admin rendering: runtime, so they hot-reload and can be overridden without recompiling.
5. **Async-first, sync-feeling.** Tokio underneath; the developer writes `.await` and nothing else async-flavored unless they want to.
6. **One binary out.** `cargo build --release` produces a single deployable artifact containing templates, static files, and migrations. Deployment story: copy one file.
7. **Boring, stable, documented.** Django won by being boring in the best way. Semantic versioning, deprecation policies from day one, docs written alongside code — never after.
8. **Regulated-industry-grade by default.** Audit logging, strong password hashing, CSRF/XSS/SQLi protection, decimal money types, explicit transaction isolation — defaults, not add-ons.

---

## Part 1 — Study the field first (1–2 weeks of reading, do not skip)

You are not the first to attempt pieces of this. Steal every good idea, note every mistake.

| Project | What to study | What to take / avoid |
|---|---|---|
| **Django itself** | App registry, migration autodetector, admin internals (`ModelAdmin`, `ChangeList`), `QuerySet` lazy evaluation, middleware contract, `settings` object | The blueprint. Read `django/db/migrations/autodetector.py` and `django/contrib/admin/options.py` line by line. |
| **cot.rs** (formerly Flareon) | Explicitly "Django for Rust." Study their model macro, admin, and what stalled or feels un-Django | Closest prior art. Learn why it hasn't taken over. |
| **Runique** | Newer Django-inspired framework (v2.x, solo author) built on Axum + SeaORM + Tera: auto admin, forms, migrations, auth, CSRF/CSP | Validates the demand. Study its admin generation and where gluing SeaORM/Tera (vs. owning the stack) limits Django-parity. |
| **loco.rs** | "Rails for Rust." Scaffolding CLI, generators, project layout, their SeaORM integration | Best-in-class Rust DX for full-stack. Their `cargo loco` CLI is the bar to beat. |
| **SeaORM / Diesel / sqlx** | SeaORM's runtime entity metadata; Diesel's compile-time DSL and its ergonomic ceiling; sqlx's driver layer and compile-checked queries | Build Djangors's ORM *on sqlx's driver layer*; do not write postgres wire protocol yourself. Avoid Diesel's trait-error ergonomics. |
| **axum / actix-web / hyper / tower** | Router design, extractors, `tower::Service`/`Layer` middleware model | Build the HTTP kernel on **hyper + tower**. Tower's `Layer` is the middleware contract; don't invent one. |
| **MiniJinja / Tera / Askama** | MiniJinja: runtime Jinja2, embeddable, by the Flask author. Askama: compile-time templates | Use **MiniJinja** as the engine core (runtime = hot reload + admin overrides), add Django-flavored filters/tags on top. |
| **Django REST Framework** | Serializers, viewsets, routers, permissions, throttling | The Phase-8 blueprint. |
| **django-simple-history, django-guardian, django-allauth, django-otp** | Audit trails, object-level permissions, social auth, 2FA | These become `djangors-contrib` crates — they're what banking/school buyers actually need. |
| **evcxr, bacon, cargo-watch, subsecond/dioxus hot-reload** | Rust dev-loop speed tooling | Feeds the `djangors run` watch/rebuild design. |

**Deliverable:** a `docs/prior-art.md` with notes, and a list of ideas explicitly adopted/rejected with reasons.

### 1.5 — Competitive reality check (as of July 2026) and how Djangors stays the most refined

Verified state of the field — nobody has shipped a *complete* Django yet; the gap is real but closing, so refinement is a moving target you must actively defend:

| Rival | Where they are | Their gap = Djangors's edge |
|---|---|---|
| **Cot** | Django-inspired, own ORM + auto-migrations + admin. Publicly early-stage: by its own admission the ORM is limited, migration autodetection covers a small subset of operations, the admin lacks pagination/filtering/search, and there's no permission system, background tasks, or WebSockets. Getting ecosystem attention (covered by JetBrains' Rust blog and InfoWorld). | Djangors's migration engine (full autodetector with state replay), full permission/auth stack, and an admin with changelist filters/search/inlines/actions is precisely the 80% Cot hasn't built. Their ORM depth is the bar to clear *first*. |
| **Runique** | v2.x, solo-author, Django-inspired glue over Axum + SeaORM + Tera: auto admin, forms/validation, migrations, auth, CSRF/CSP. | Gluing third-party ORM/templating caps Django-parity: no `field__lookup` querysets, no app registry, no `makemigrations`-style autodetection against its own metadata, thin docs, bus-factor 1. Djangors owning the ORM metadata layer (ModelMeta) is what unlocks the admin/forms/migrations trinity Runique can't fully deliver on SeaORM. |
| **loco.rs** | The most polished Rust full-stack DX today (Rails-flavored): great CLI/generators/workers/mailers on SeaORM. | Rails-shaped, not Django-shaped: no real admin, no Django-style migrations-from-models, weaker forms story. Its CLI polish is the standard Djangors's `djangors` binary must match. |
| **rustango** | Discovered during name research (crates.io, repo `ujeenet/rustango`): describes itself as Django-shaped batteries-included — ORM, migrations, auto-admin, multi-tenancy, audit log, auth (sessions/JWT/OAuth2), ViewSets + OpenAPI, jobs, email, S3 media, CSRF/CSP/rate-limit/idempotency middleware. First published April 2026, already at v0.46 after 41 releases in ~10 weeks, but ~680 total downloads and near-zero visibility. | The most plan-overlapping rival on paper — but 41 releases in 10 weeks with no adoption suggests breadth-first/possibly generated code over depth and community. **Do a proper teardown in week 1** (clone it, run its admin, try its migrations on a 20-edit model history): steal what works, and let its weaknesses (docs, tests, API stability, trust) define where Djangors out-executes. Also a warning: the space is being actively land-grabbed *right now* — reserve names and ship visibly early. |

**Standing discipline to stay #1 in refinement (make it a process, not a one-time check):**
1. **Quarterly competitive teardown** — re-run the Cot/Runique/loco feature matrix every release cycle; file a Djangors issue for any feature where a rival is ahead. Keep the matrix public in `docs/comparison.md` (honest comparisons build trust and rank on search).
2. **Refinement is depth, not feature count.** The differentiators no rival has *together*: full migration autodetection, an ops-team-usable admin, object-level permissions + audit + 2FA (the regulated-industry stack, Part 6), a DRF-equivalent with generated OpenAPI, background tasks, and Django-tutorial-quality docs. Rivals each have 2–3 of these at partial depth; Djangors's bar is all of them at production depth.
3. **Watch the watchers:** subscribe to Cot's and Runique's releases, This Week in Rust, and r/rust — respond to their launches with Djangors demo posts, not arguments.

---

## Part 2 — Foundational decisions (make them now, in writing)

| Decision | Choice | Rationale |
|---|---|---|
| Language edition / MSRV | Latest stable edition; MSRV = stable-minus-2, documented and CI-enforced | Serious orgs need MSRV guarantees. |
| License | MIT OR Apache-2.0 (dual) | Rust ecosystem standard; enterprise-safe. |
| Async runtime | Tokio, non-optional | Fighting runtime-genericity costs years. Ship on Tokio. |
| HTTP layer | hyper + tower + tower-http | Battle-tested; middleware = `tower::Layer`. |
| DB driver layer | sqlx (Postgres first, then SQLite, then MySQL/MariaDB) | Async, pure Rust, connection pooling included. Postgres first because banks/e-commerce run it; SQLite second because the tutorial and tests need zero-setup. |
| Template engine | MiniJinja core + Djangors filter/tag pack + template-dir loader with override precedence (app dirs → project dir), embedded via `include_dir!` in release | Runtime rendering is what makes the admin skinnable and dev loop instant. |
| Serialization | serde everywhere | Obvious. |
| Time | `chrono` + first-class timezone support (`USE_TZ = true` equivalent, store UTC) | Django got time handling right; copy it. |
| Money / decimals | `rust_decimal` as the default `DecimalField` backing type | Banking requirement. Floats for money = disqualified. |
| Password hashing | argon2id default; bcrypt/pbkdf2 verifiers for import-migration from Django apps | Also enables *migrating a real Django app to Djangors without password resets* — a killer adoption feature. |
| IDs | i64 autoincrement default, UUIDv7 opt-in per model or per project | Match Django default; offer the modern option. |
| Error model | `djangors::Error` with typed kinds; every error page in dev shows the Django-style yellow debug page (request info, template context, SQL log, backtrace) | The Django debug page is beloved. Clone it unapologetically. |
| Config | `DjangorsSettings` struct built from: defaults ← `djangors.toml` ← environment profile (`[profile.production]`) ← env vars ← code overrides | 12-factor-friendly, still one obvious place to look. |
| Name | **Djangors** — crate `djangors`, verified available on crates.io as of 2026-07-17 (`rango` and `rjango` are both taken by others). **Publish a 0.0.1 placeholder of `djangors` + `djangors-orm`, `djangors-admin`, `djangors-cli`, `djangors-macros` etc. TODAY** — name-squatting on Django-for-Rust names is demonstrably happening. Grab the GitHub org and a domain (djangors.rs / djangors.dev). CLI binary ships as `dj` (typed constantly: `dj new`, `dj run`) with `djangors` as an alias. **Considered and rejected: `django` / `django.rs`** — the `django` crate name is technically unclaimed, but "Django" is a registered trademark of the Django Software Foundation; naming a competing framework literally "Django" invites a takedown request and permanently brands the project as a clone rather than its own thing. If the DSF-adjacent branding matters, email the DSF trademark committee for written permission before using it — otherwise "Djangors" signals the lineage while remaining a distinct mark | An unclear name story at launch is a self-inflicted wound; settle it in week 1 and never revisit. |

---

## Part 3 — Architecture: the workspace

A single Cargo workspace. Users depend on the `djangors` facade crate and get everything via feature flags (all-on by default — batteries included).

```
djangors/
├── Cargo.toml                  # workspace
├── crates/
│   ├── djangors/                  # facade: re-exports, prelude, feature flags
│   ├── djangors-core/             # HTTP kernel: request/response, routing, middleware,
│   │                           # handlers, extractors, error pages, signals bus
│   ├── djangors-macros/           # ALL proc macros: #[derive(Model)], #[djangors::main],
│   │                           # q!(), urls!, #[handler], form derives
│   ├── djangors-db/               # backend abstraction over sqlx: connections, pools,
│   │                           # transactions, SQL dialect layer
│   ├── djangors-orm/              # Model trait, ModelMeta, QuerySet, managers,
│   │                           # relations, aggregation, expressions (F/Q objects)
│   ├── djangors-migrations/       # migration engine: state graph, autodetector,
│   │                           # operations, executor, squashing
│   ├── djangors-forms/            # Form/ModelForm, fields, widgets, validation, rendering
│   ├── djangors-template/         # MiniJinja integration, Django-flavored tags/filters,
│   │                           # loader precedence, {% url %}, {% static %}, {% csrf_token %}
│   ├── djangors-auth/             # User model, groups, permissions, backends, hashing,
│   │                           # login/logout views, decorators/guards
│   ├── djangors-sessions/         # session engine: signed-cookie, db, cache backends
│   ├── djangors-admin/            # THE admin: registry, changelist, forms, inlines,
│   │                           # actions, filters, search, theming
│   ├── djangors-staticfiles/      # collectstatic, hashed filenames, embed-in-binary,
│   │                           # dev serving
│   ├── djangors-cache/            # cache framework: in-memory, redis, db backends;
│   │                           # per-view + template-fragment caching
│   ├── djangors-mail/             # email backends: SMTP, console, file, in-memory (tests)
│   ├── djangors-i18n/             # translation catalogs (Fluent or gettext), locale
│   │                           # middleware, formats, {% trans %}
│   ├── djangors-test/             # test client, TestDatabase (per-test txn rollback),
│   │                           # fixtures, assertion helpers
│   ├── djangors-cli/              # the `djangors` binary: new/generate/run/etc.
│   ├── djangors-rest/             # DRF equivalent: serializers, viewsets, routers,
│   │                           # OpenAPI generation (Phase 8)
│   ├── djangors-channels/         # WebSockets/SSE, background workers, task queue (Phase 8)
│   └── contrib/
│       ├── djangors-contrib-messages/     # flash messages
│       ├── djangors-contrib-audit/        # model history / audit log (simple-history)
│       ├── djangors-contrib-guardian/     # object-level permissions
│       ├── djangors-contrib-otp/          # TOTP/WebAuthn 2FA
│       ├── djangors-contrib-sitemaps/
│       ├── djangors-contrib-syndication/  # RSS/Atom
│       ├── djangors-contrib-humanize/
│       └── djangors-contrib-flatpages/
├── examples/
│   ├── polls/                  # the tutorial app, always compiling in CI
│   ├── ecommerce/              # showcase: catalog, cart, orders, admin-heavy
│   └── school/                 # showcase: students, grades, RBAC, audit
├── docs/                       # mdBook or Astro Starlight site source
├── benchmarks/                 # criterion micro + TechEmpower-style macro benches
└── tools/                      # xtask: release automation, MSRV check, docs deploy
```

### The app system (Django's secret weapon, ported)

A Djangors **app** is a crate (or module) implementing `AppConfig`:

```rust
pub struct PollsApp;

impl AppConfig for PollsApp {
    const LABEL: &'static str = "polls";
    fn models(&self) -> Vec<&'static ModelMeta> { djangors::models_of!(polls) }
    fn urls(&self) -> Router { urls() }
    fn migrations(&self) -> &'static [&'static dyn Migration] { migrations::ALL }
    fn templates(&self) -> Option<TemplateSource> { Some(embed_templates!("templates")) }
    fn admin(&self, site: &mut AdminSite) { admin::register(site); }
}
```

The project's settings list installed apps; the **app registry** (built at startup) is what powers the admin, `makemigrations`, `{% url %}` reversing, and management commands — exactly Django's architecture, with the registry populated by proc-macro-generated metadata instead of Python introspection.

### What developer code looks like (write this before writing the framework)

**Author `examples/polls` FIRST, as aspirational code that doesn't compile yet.** This is your API spec. Every framework feature exists to make this file compile and run.

```rust
// models.rs
use djangors::prelude::*;

#[derive(Model)]
#[djangors(app = "polls", ordering = "-pub_date")]
pub struct Question {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 200)]
    pub question_text: String,
    #[djangors(verbose_name = "date published", db_index)]
    pub pub_date: DateTime<Utc>,
}

#[derive(Model)]
#[djangors(app = "polls")]
pub struct Choice {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(to = Question, on_delete = "cascade", related_name = "choices"))]
    pub question: ForeignKey<Question>,
    #[djangors(max_length = 200)]
    pub choice_text: String,
    #[djangors(default = 0)]
    pub votes: i32,
}

// views.rs
#[handler]
async fn index(req: Request) -> Result<Response> {
    let latest = Question::objects()
        .filter(q!(pub_date__lte = Utc::now()))
        .order_by("-pub_date")
        .limit(5)
        .all(req.db())
        .await?;
    render(&req, "polls/index.html", context! { latest_question_list => latest })
}

#[handler]
async fn vote(req: Request, Path(question_id): Path<i64>) -> Result<Response> {
    let question = Question::objects().get_or_404(req.db(), question_id).await?;
    let choice_id: i64 = req.form().await?.require("choice")?;
    Choice::objects()
        .filter(q!(question = question.id, id = choice_id))
        .update(req.db(), set!(votes = F("votes") + 1))   // race-safe, like Django's F()
        .await?;
    redirect(reverse!("polls:results", question_id))
}

// urls.rs
pub fn urls() -> Router {
    Router::app("polls")
        .route("", get(index), "index")
        .route("{question_id}/", get(detail), "detail")
        .route("{question_id}/vote/", post(vote), "vote")
}

// admin.rs
pub fn register(site: &mut AdminSite) {
    site.register::<Question>(
        ModelAdmin::new()
            .list_display(&["question_text", "pub_date", "was_published_recently"])
            .list_filter(&["pub_date"])
            .search_fields(&["question_text"])
            .inlines(&[Inline::<Choice>::tabular().extra(3)]),
    );
}

// main.rs
#[djangors::main]
async fn main() -> Result<()> {
    Djangors::new(settings())
        .app(PollsApp)
        .run()          // also dispatches CLI subcommands: migrate, createsuperuser…
        .await
}
```

---

## Part 4 — The five hard problems (solve on paper before coding)

These are where "Django for Rust" attempts die. Each gets a design doc in `docs/design/` before implementation.

### 4.1 Model metadata without reflection
Python's admin/migrations/forms run on runtime introspection. Rust has none.
**Solution:** `#[derive(Model)]` generates, per model: (a) typed field accessors and a `Fields` struct for query building; (b) a `&'static ModelMeta` — field names, column types, verbose names, validators, widget hints, relations — registered into the app registry. `ModelMeta` is the single source of truth consumed by migrations, admin, forms, and serializers. This is the keystone crate; over-invest here.

### 4.2 QuerySet ergonomics
Django's `filter(pub_date__lte=now, author__name__icontains="x")` is stringly-typed magic.
**Solution:** the `q!()` proc macro — Django's exact lookup syntax (`field__lookup`, `relation__field`), but *validated at compile time* against the generated `Fields` metadata: typo'd field or wrong value type = compile error. Under the hood it builds the same expression tree as the fluent typed API (`Question::fields().pub_date().lte(now)`), which also exists for people who want pure-Rust style and IDE completion. Support: `Q` combinators (`q!(a) | q!(b)`, `!q!(c)`), `F` expressions, `annotate`/`aggregate` (Count/Sum/Avg/Min/Max), `select_related` (JOIN) / `prefetch_related` (second query), `values()`/`values_list()` (row structs), slicing, `exists`, `count`, `update`, `delete`, `bulk_create`, `get_or_create`, `iterator` (streaming). Lazy: a `QuerySet` is a pure value until `.all()/.get()/.first()` executes it — chainable and reusable exactly like Django's.

### 4.3 Migrations with autodetection
`makemigrations` must diff "models as written" against "state after all existing migrations."
**Solution (Django's, transplanted):** migrations are Rust files declaring `Operation` lists (`CreateModel`, `AddField`, `AlterField`, `RunSQL`, `RunRust`), each operation able to mutate an in-memory `ProjectState`. The autodetector replays all migrations to build historical state, gets current state from the app registry (`ModelMeta`), diffs, and emits a new migration file (generated Rust source, `cargo fmt`-ed). Because migrations live *in the compiled binary*, `app migrate` works on a production box with no source tree. Include from day one: dependency graph between apps' migrations, `sqlmigrate` (print SQL), `--check` for CI, `--fake`, squashing, and data migrations via `RunRust` closures with a lightweight historical-model API (document its limits honestly — this is Django's hairiest corner; it's acceptable for v1 to give data migrations the *current* model API with loud docs).

### 4.4 The admin without a dynamic language
**Solution:** the admin is a runtime engine over `ModelMeta` + `ModelAdmin` config: it builds changelists (pagination, sorting, filters, search, date hierarchy, bulk actions), change forms (via djangors-forms, from field metadata + widget hints), inlines, delete-confirmation with related-object collection (mirror Django's `NestedObjects`), history integration, and permission checks — all rendered through overridable MiniJinja templates. Ship a clean, modern default theme (server-rendered + sprinkles of vanilla JS/htmx-style progressive enhancement; **no SPA framework dependency**). "Angular-level management" is achieved by making the admin genuinely rich: saved filters, CSV/XLSX export action built-in, column choosing, dark mode, mobile-usable. Admin URLs, templates, and forms all overridable per-model — parity with Django's extension points (`get_queryset`, `readonly_fields`, custom actions, custom admin views).

### 4.5 The dev loop (Rust compiles; Django reloads)
This is the #1 DX risk. Attack it from five directions:
1. **Runtime templates + static files** — the majority of iteration (HTML/CSS) needs zero recompile; `djangors run` watches and the browser auto-reloads (built-in livereload websocket in dev).
2. **`djangors run`** = cargo-watch equivalent: watches source, rebuilds, restarts, holds requests during restart (no connection-refused flashes), preserves sessions (signed cookies survive restarts).
3. **Fast dev profile out of the box:** generated projects ship a tuned `[profile.dev]` (opt-level 0, incremental, optional cranelift backend, optional mold/lld linker) — target: **sub-3-second rebuild** for handler-level edits on the tutorial app. Measure this in CI as a regression test.
4. **Workspace splitting guidance:** generators put apps in separate crates so touching one app recompiles one crate.
5. **Debug error page** with full context (Django's yellow page), SQL query log panel, and a debug-toolbar contrib crate later.

---

## Part 5 — Phased build plan

Phases are sequential dependencies, not calendar promises. Each phase has a **Definition of Done** (DoD) and ends with the polls example exercising every new feature. Rough effort assumes 1–2 experienced Rust devs; scale accordingly.

### Phase 0 — Bootstrap (days, not weeks)
- [x] `git init`; workspace `Cargo.toml`; empty crates for core/macros/orm/db/cli with `#![deny(missing_docs)]` policy decided. **Done** — 32 crates under `crates/`, all at `#![deny(missing_docs)]` since 9.5.
- [x] Reserve crates.io names, GitHub org, domain, chat server. Check `djangors` name availability/conflicts *first*; have a backup name. **Done** — all 32 crates published, repo at `github.com/QUANTORA-ltd/djangors`, site at `djangors.vercel.app`. **Remaining:** no chat server (Discord/Zulip) exists; no `djangors.rs`/`djangors.dev` domain, the site is on a `vercel.app` subdomain.
- [x] CI: fmt, clippy (deny warnings), test, docs build. **Done, but as CircleCI, not GitHub Actions** — `.circleci/config.yml`, two jobs: `test` (`fmt --check` → `clippy --workspace --all-targets -D warnings` → `build --workspace --all-targets` → `cargo test --workspace` against a `cimg/postgres:16.0` service container → `cargo doc --workspace --no-deps` with `RUSTDOCFLAGS: -D warnings`) and `audit` (`cargo audit`, with three RUSTSEC ignores each carrying an inline comment naming the advisory and why it is unfixable upstream). **Remaining, and all still genuinely open:** (a) no OS/toolchain matrix — linux + the image's pinned 1.83 only, no mac, no windows, no MSRV leg; (b) **no `rust-version` declared in the workspace `Cargo.toml` at all**, so there is no MSRV to test against and a dependency bump can raise the required toolchain for downstream users silently; (c) no `cargo-deny` (license checking is not covered — `cargo audit` covers advisories only); (d) no examples-build step; (e) **CI never exercises SQLite** — see the Phase 2 test-harness item below.
- [x] `docs/design/` RFC template; write RFCs for the five hard problems (Part 4). **Done** — 99 design docs under `docs/design/`; every phase since has been design-doc-first.
- [x] Write the aspirational `examples/polls` (Part 3) — the API spec. **Done** — `examples/polls` is real and passes its own tests; `examples/school` added later as the larger dogfooding target.
- [ ] LICENSE (MIT/Apache-2.0), CODE_OF_CONDUCT, CONTRIBUTING, SECURITY.md (with a security-report email), README with the pitch. **Licensing and README are done** — `LICENSE-MIT` + `LICENSE-APACHE` (standard Rust dual-license) and a `README.md` with the pitch and a License section. **Still missing, confirmed against `git ls-files`:** `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, and `SECURITY.md` — there is no `.github/` directory either. `SECURITY.md` is the one that actually costs something: there is deep security *content* in the repo (`docs/design/security-checklist.md`, `docs/security-review-2026-07-27.md`, `docs/src/guides/security.md`), but no documented private disclosure channel, so a researcher who finds an auth bug has nowhere to send it except a public issue. That is a ~15-line file, and it is the cheapest credibility item on the whole plan.

**DoD:** `cargo build` green in CI on empty crates; polls example exists as spec; 5 design docs drafted.

### Phase 1 — HTTP kernel (djangors-core) (~4–6 weeks)
- [x] Request/Response types over hyper; body handling, multipart, form parsing, JSON, file uploads. **Done** — `djangors-core`'s `Request`/`Response`, multipart via `multer` with `Constraints::size_limit` (whole-stream + per-field). **Remaining:** uploads are **not** streamed to temp files — `multer` buffers in memory and the only defence is the size limit, so a large-file upload path is bounded rather than streaming.
- [x] Router: Django-style path syntax (`{id}`, converters `{id:i64}`, `{slug:slug}`), app namespacing, include/nesting. **Done** — `router.rs`, `CaptureType` covers `i64`/`slug`/plain segments. **Remaining, and never started: named routes with `reverse!()`.** There is no route-name registry and no reverse lookup anywhere in the workspace. This is a bigger gap than it looks: it is also why `{% url %}` does not exist (see Phase 3), and it means every link in user code is a hardcoded string.
- [x] Handler trait + extractors — axum-familiar but Django-flavored. **Done for `Json`, `Query`, `Form`** (`extract.rs`) plus `PathParams` from the router. **Remaining:** no `State` extractor; handlers reach shared state by other means.
- [x] Middleware = tower `Layer`. **Done** — `logging_layer()` (`tower_http::trace::TraceLayer`), `SecurityHeadersLayer`, `HstsLayer`, `CspLayer`, `RequestIdLayer`, `CsrfLayer`, `HostValidationLayer`, `compression_layer()` (gzip + brotli), and trailing-slash normalization equivalent to Django's `CommonMiddleware`.
- [x] Settings system (Part 2 design) + `runserver` bound into a `Djangors` application object that owns the app registry. **Done** — `DjangorsSettings` + `validate()`, `inventory`-based app/model registry; `#[derive(Settings)]` added later in Phase 12.
- [x] Error handling: `Result<Response>`, Http404/Http403/Http400 types, custom error handlers, the **dev debug page**, production error pages. **Done** — `DjangorsError`; `debug_page::render_debug_page` was moved to `#[doc(hidden)]` in the 10.13 freeze review because it is dev-only and security-sensitive.
- [x] Signals bus (typed, async): request_started/finished, plus the plumbing model signals use. **Done** — `signals.rs`, `Signal<T>` with `connect()`, `RequestStarted`/`RequestFinished`.
- [x] Structured logging via `tracing` with pretty dev output. **Done.**

**DoD:** a hello-world Djangors app with routing, middleware, templates NOT yet — plain responses; benchmarked ≥ axum-minus-10% throughput.

### Phase 2 — ORM + migrations (djangors-db, djangors-orm, djangors-migrations, djangors-macros) (~3–4 months; the long pole)
- [x] djangors-db: sqlx integration, pool config in settings, `DATABASES`-style multi-db support with `.using("replica")`, transactions, explicit isolation levels. **Done** — `Database`, `DatabaseConfig`, `transaction`/`transaction_with_isolation`, and (13.3) `transaction_conn` for the dialect-agnostic path. **Remaining: savepoints were never implemented** — there is no `SAVEPOINT` anywhere in the workspace, so nested/partial rollback inside a transaction is not available. That matters for the banking pitch specifically, since Part 6 item 2 names savepoints as acceptance criteria.
- [x] `#[derive(Model)]` + `ModelMeta` (design 4.1). **Done.** Field types: all Django fields — Char/Text/Integer(s)/BigInt/Float/**Decimal**/Boolean/Date/DateTime(tz-aware)/Time/Duration/UUID/Email/URL/Slug/IP/Binary/JSON/File/Image; null/blank/default/choices/unique/db_index/validators/help_text/verbose_name; Meta: table name, ordering, unique_together, indexes, constraints (CheckConstraint!), permissions.
- [x] Relations: ForeignKey, OneToOne, ManyToMany, reverse accessors, related managers. **Done at the metadata + query level** (`meta.rs`). **Remaining:** `on_delete` is still largely metadata-only — only `Protect` is enforced, and only as an admin-layer pre-check, not a schema constraint; `Cascade`/`SetNull`/`Restrict`/`DoNothing` are declared but not enforced by the ORM or the generated DDL (carried from the Phase 5 delete-confirmation item).
- [x] QuerySet engine (design 4.2) incl. aggregation, expressions, raw SQL escape hatch, `select_related`/`prefetch_related`. **Done** — `queryset.rs`, every identifier quoted since 12.1. **Remaining, all three never started:** `Exists`/`OuterRef` correlated subqueries (no `OuterRef` symbol exists in the workspace), and **`select_for_update` — the "(banking!)" item.** The only `FOR UPDATE` SQL anywhere is a hardcoded `FOR UPDATE SKIP LOCKED` inside `djangors-tasks`' claim query; there is no QuerySet-level row-locking API a user can call. Part 6 item 2 lists this as acceptance criteria for the banking pitch, so it is a headline gap, not a nicety.
- [x] Model lifecycle: `save()` (insert-or-update semantics), `delete()` with cascade collection, `full_clean()` validation, pre/post save/delete signals. **Done.**
- [x] Migration engine (design 4.3): operations, state replay, autodetector, executor, `djangors_migrations` table. **Done**, and dialect-aware since 13.3. **Remaining — the CLI verb set is much thinner than this line claims:** `dj` ships `migrate` (with `--rollback`) and `makemigrations` (with `--check`). There is **no `sqlmigrate`, no `showmigrations`, no `--fake`, and no `--plan` flag.** `sqlmigrate` and `showmigrations` are the two a Django user reaches for first when a migration misbehaves, and neither exists.
- [x] Backends: **Postgres** complete → **SQLite** complete. **Done (Phase 13)** — ORM, all production raw SQL, and migrations/`makemigrations` are dialect-aware; released in 0.5.0 and completed in 0.6.0. **Remaining:** MySQL, still correctly deferred to after 1.0. Note the SQLite `ALTER TABLE` story is honest but partial — `AlterColumnType` returns `MigrationError::UnsupportedOnDialect` on SQLite rather than performing Django's 12-step table rebuild.
- [ ] Test harness for the ORM itself: run the full ORM suite against real Postgres + SQLite in CI (containers). **Half done, and this is the single cheapest open item in the whole plan.** The *harness* supports both backends as of 14.1 (`TestBackend::{Postgres, Sqlite}`, `TEST_BACKEND=sqlite`, in-memory SQLite pinned to `max_connections(1)`); `djangors-admin`'s 32 tests run in 0.69s on SQLite vs 15.8s on Postgres. But **CI only ever runs the Postgres leg** — `.circleci/config.yml` sets `DATABASE_URL`, so the harness silently takes the Postgres arm and the dual-backend support shipped in 0.6.0 is verified by hand on one developer machine and by nothing else. Fix is one extra job running `env -u DATABASE_URL TEST_BACKEND=sqlite cargo test --workspace`, needing no service container. **Second, smaller caveat to carry:** five tests (isolation levels, advisory locks, `SKIP LOCKED`) early-return when `DATABASE_URL` is unset and **report as passing on SQLite without executing**, so a green SQLite run must never be read as covering those paths.

**DoD:** polls models work end-to-end: makemigrations → migrate → query in views; a torture-test app with every field type and relation migrates cleanly forward from 20+ sequential model edits.

### Phase 3 — Templates, forms, static files (~6–8 weeks)
- [x] djangors-template: MiniJinja engine; loader precedence; auto-escaping on; Django filter pack; template inheritance native. **Done for the engine, loader, escaping, and filters** — `date`, `floatformat`, `pluralize`, `truncatewords`, `intcomma`, `filesizeformat`, `naturaltime`, `trans`. **Remaining: none of the custom tags exist.** `djangors-template` registers no globals or functions at all, so `{% url %}`, `{% static %}`, `{% csrf_token %}`, and `{% now %}` are all absent. `{% url %}` is blocked upstream on named routes/`reverse!()` never being built (Phase 1); the other three are not blocked on anything. In practice the admin works around this by passing values through context, which is why the gap survived this long.
- [ ] Dev: template hot reload. Release: `include_dir!` embedding. **Not done.** No hot-reload/auto-reload path exists; `include_dir` appears only in `djangors-admin` for its own bundled templates, not as a general release-embedding option for user projects.
- [x] djangors-forms: `Form` derive + `ModelForm` (generated from ModelMeta); field validation pipeline (`clean_<field>` hooks, `clean()` cross-field), typed `cleaned_data`, error rendering, file-upload fields. **Done** — `fields.rs`, `error.rs`. **Remaining, and a bigger slice than the line suggests: there is no widget layer at all.** No `Widget` type, no select/radio/checkbox/date-picker widgets, and no `as_div`/`as_table`/`as_p` renderers — form HTML is produced by the admin's own templates, not by the forms crate. **Formsets do not exist either**, which is precisely why admin inlines (tabular/stacked) are still open in Phase 5.
- [x] djangors-staticfiles: dev serving, `collectstatic`, hashed-manifest storage (cache-busting), embed-in-binary option. **Done**, plus an S3 storage backend added in Phase 10.

**DoD:** polls has real templates and a hand-written form with validation errors rendering; static CSS served in dev and embedded in release.

### Phase 4 — Sessions, auth, security (~6–8 weeks) — *the banking phase*
- [x] djangors-sessions: engines — signed-cookie (default dev), database, cache; rotation, expiry, `session.set/get` typed API. **Done.**
- [x] CSRF middleware: token generation, header validation **(done)**, form-field (`csrfmiddlewaretoken`) fallback validation **(done, core-level — 5.8.9, commit `8a448db` — and wired into every admin `<form>` — 5.8.10, commit `b2c5d16`)**, exempt marker. On by default. **Remaining: no `{% csrf_token %}` template tag** (see Phase 3) — user templates must thread the token through context by hand, which is a real ergonomic and footgun gap versus Django.
- [x] Security middleware: HSTS, X-Content-Type-Options, Referrer-Policy, CSP helper, secure/httponly/samesite cookie defaults, host header validation (`ALLOWED_HOSTS`). **Done** — `HstsLayer`, `SecurityHeadersLayer`, `CspLayer`, `HostValidationLayer`. The 10.x security review found and fixed both example apps failing to set `Secure` on CSRF/session cookies.
- [x] djangors-auth: `User` model (swappable via setting, like `AUTH_USER_MODEL` — design this in *now*, Django learned the hard way it can't be retrofitted), groups, model-level permissions (auto add/change/delete/view per model), authentication backends trait, argon2id hashing + legacy verifiers, login/logout/password-change/password-reset views + email flow, `login_required` guard/extractor (`Auth<User>` extractor pattern), permission guards, session fixation protection (rotate on login).
- [x] Rate limiting middleware (login throttling default-on), constant-time credential comparison, audit hooks for login/logout/failed-login signals. **Done** — the security review independently confirmed a real dummy-hash verification runs even for nonexistent usernames (genuine timing-attack mitigation, not just a claim) and constant-time CSRF/session signature comparison.
- [x] Security review checklist doc; fuzz targets for parsers (cookies, query strings). **Done** — `docs/design/security-checklist.md` is a full 147-line OWASP Top 10 self-assessment (A01–A10 plus a parser-robustness section), which also satisfies this phase's DoD. `docs/security-review-2026-07-27.md` is the later internal review. Fuzz targets cover `cookie_header`, `query_string`, and `session_cookie_decode`, with empty `fuzz/artifacts/` (no crashes found).

  **Remaining, and the first item is newly actionable rather than long-known:**
  1. **A multipart fuzz target.** The checklist justified its absence with "no multipart parser exists yet — nothing to fuzz there." **That justification is now stale**: `djangors-core`'s `extract.rs` parses multipart via `multer`. So the one parser handling the largest, least-structured attacker-controlled input is the only attacker-facing parser with no fuzz coverage — and per Phase 1 it buffers rather than streams, which is exactly the shape where a resource-exhaustion bug hides.
  2. **Threat model doc** — never written.
  3. Fuzzing is a local one-off smoke run per target, not wired into CI or OSS-Fuzz, so the corpus only grows when someone runs it by hand.

  **`docs/design/security-checklist.md` needs its own staleness pass.** Its "summary of open gaps" still lists as open several things since fixed: CSRF form-body validation (done, 5.8.9/5.8.10), the `Secure` cookie flag (wired in both examples during the 10.x review), and groups/per-model permissions (done, 5.7.1). A security doc that overstates its gaps is safer than one that understates them, but it is still wrong, and it is the document an external auditor would read first.

**DoD:** polls requires login to vote; full password-reset flow works via console email backend; OWASP top-10 self-assessment written.

### Phase 5 — THE ADMIN (~3–4 months, worth every week)

> **Status (2026-07-18):** DoD met (5.9). Every top-level bullet below has now landed at least a
> "done (v1)" pass, including the full 5.8 arc (theming/history/extension points, 5.8.1–5.8.12).
> What remains across the board is optional, non-blocking follow-up work (choice-field filters,
> autocomplete/M2M widgets, inlines, full `on_delete` enforcement beyond `Protect`, generic
> `AuthUser` support) — see `docs/design/phase-5-roadmap.md` for the authoritative slice-by-slice
> status, commit references, and the full deferred-items ledger.

- [x] AdminSite + registry; auto-URLs; login gated by `is_staff`. *(5.1, commit 9edf249 — v1 constraint: gate hardcoded to the built-in `User`, not generic `AuthUser`)*
- [x] Changelist: **done:** all-fields columns, sorting, pagination *(5.2, commit ac647f9)*; `list_display` (real-field subset/reorder) + `search_fields` (ILIKE search box) via new `AdminSite::register_with()` customization API *(5.6.1, commit a6ad28f)*; `list_filter` v1, Boolean fields only *(5.6.2, commit cba2109 — choices-based filtering blocked on `FieldMeta` gaining choices metadata, which doesn't exist yet)*; bulk delete, single hardcoded action with two-step confirm *(5.6.3, commit 47459ee)*; `date_hierarchy` v1, one `DateTime` field, year/month/day drilldown, drilldown counts not combined with search/`list_filter` state *(5.6.4, commit a744250)*; `list_editable` v1, text/numeric fields only, shares the bulk-delete `<form>` via per-button `formaction` (no JS, no nested forms) *(5.6.5, commit 27bce9e)*; CSV export v1, plain `GET` link exporting the current filtered/searched/ordered queryset (not a selected-rows action), hand-rolled RFC 4180 escaping, no new dependency *(5.6.6, commit 6f49e30)*; `list_display` computed-method columns (`ModelAdminConfig.computed_columns`, a function of the row's field values, checked before falling back to a real field) and a real bulk-actions dispatch mechanism for selected-rows actions (`AdminAction` + `POST bulk-action/` with a two-step confirm page, same `formaction`-per-button pattern 5.6.5 established) *(5.8.12, commit `8d0c683`)*. **remaining:** `list_filter` for choice fields (blocked as above), saved views.
- [x] Change form: **done (v1):** add + edit pages, all fields, server-side validation with all-errors-at-once reporting, changelist row links *(5.4, commit 9f016fe)*; fieldsets (rows grouped under `<fieldset><legend>` in registration order), `readonly_fields` (rendered as inert text), `raw_id_fields` (a plain lookup link to the related model's changelist rather than a full `<select>`) *(5.8.12, commit `8d0c683`)*. **remaining:** autocomplete FK widgets with an async search endpoint (raw_id_fields v1 is a static link, not a live-search picker), M2M widgets, inlines (tabular/stacked) via formsets. *(CSRF header-only gap fixed at the core-middleware level, 5.8.9, commit `8a448db`, and the admin's own `<form>` elements now emit the hidden `csrfmiddlewaretoken` field, 5.8.10, commit `b2c5d16` — a raw browser form POST works end-to-end without client-side JS)*
- [x] Delete confirmation: **done (v1):** confirm page + delete action, one-hop related-object counts via the `inventory`-based global model registry, `on_delete` shown for information only *(5.5, commit aba0ff9)*; permission enforcement — `require_perm` gates GET/POST delete and bulk-delete on the `delete_{model}` codename, superusers bypass *(5.7.2, commit b9aed08)*; transitive (multi-hop, depth-bounded + cycle-safe) related-object walk, and `on_delete: Protect` enforcement — single-object delete now 400s and bulk delete silently skips protected pks instead of deleting them *(5.8.12, commit `8d0c683`)*. **remaining:** real DB-level (or ORM-level) enforcement for the other `on_delete` variants (Cascade/SetNull/Restrict/DoNothing are still metadata-only — see roadmap ledger); only `Protect` is enforced, and only as an admin-layer pre-check, not a schema constraint.
- [x] Permissions: **done:** `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission` models (plain FK join tables, no real many-to-many ORM support needed), `AuthUser::is_superuser()`, `has_perm(db, user_id, codename)`, `sync_permissions()` + `dj createpermissions` *(5.7.1, commit bb284a4)*; every admin view (changelist/export/add/change/list_editable save/delete/bulk-delete) now checks `has_perm` per action via `require_perm` instead of a blanket `is_staff` check, superusers bypass entirely, `admin_index` filters to only viewable models *(5.7.2, commit b9aed08)*; admin UI for groups/permissions/user assignments — `Permission`/`Group`/`UserGroup`/`GroupPermission`/`UserPermission` registered with the generic admin like any other model, real add/change/delete/changelist CRUD, no new admin-crate mechanism needed — and UI-level hiding of action buttons (`has_add/change/delete/view_permission` booleans threaded into every template context) for a user lacking the relevant permission *(5.8.12, commits `8d0c683`/`1305195`)*. **remaining:** `AuthUser`-generic (not hardcoded to `djangors_auth::User`) support, same standing limitation as the 5.1 staff gate.
- [x] History: **done (v1):** admin log entries table (`LogEntry`, one entry per successful add/change/delete, per-user "Recent actions" panel on the admin index) *(5.8.11, commit `01cd081`)*; per-object History page (`GET /{app}/{model}/{pk}/history/`, newest-first, joined against `auth_user` for the acting username) *(5.8.12, commit `8d0c683`)*. **remaining:** real field-level diffing/`change_message` beyond fixed action strings, IP/request-id capture; deep integration with djangors-contrib-audit for full record diffing.
- [x] Theming: clean modern default (CSS custom properties, dark mode — done, 5.8.7); every template overridable; per-site branding (title/heading — done, 5.8.8; logo, accent color — done, 5.8.10) via settings. *(Conversion mechanism complete as of 5.8.6 — every admin page renders through minijinja: `admin_index` (5.8.1, eb1a916), delete-confirm (5.8.2, dfe6ec0), bulk-delete confirm (5.8.3, 965fb24), `list_editable` save-error page (5.8.4, 8b7dce5), add/change form (5.8.5, 55e54b5), changelist (5.8.6, ab1363e). 5.8.7 (cdd64bf) added a real HTML5 page shell (`admin/base.html`), CSS custom properties, and automatic dark mode. 5.8.8 (ad23e39) added `SiteBranding` (`site_header`/`site_title`) threaded through `AdminSite::urls()`'s existing per-closure-snapshot mechanism into every template-rendering handler — `AdminSite::new()`'s signature unchanged, opt-in via `with_site_header`/`with_site_title` builders. 5.8.10 (b2c5d16) added `logo_url`/`accent_color` via `with_logo_url`/`with_accent_color`, same mechanism, plus default favicons served at the app root.)*
- [x] Extension points: **done (v1):** `AdminSite::extra_route()` lets a host app mount an arbitrary extra `Router` under the admin site (custom admin views); `ModelAdminConfig.base_filter` ANDs a fixed `UnresolvedExpr` into every changelist/queryset for a model (a scoped `get_queryset` override); custom actions with an intermediate confirmation page via `AdminAction`/`requires_confirm` (this slice's own bulk-actions mechanism, above); `ModelAdmin` is already a trait with `DefaultModelAdmin` providing every default *(5.8.12, commit `8d0c683`)*. **remaining:** a full `get_queryset` override (an arbitrary closure, not just a fixed `base_filter`) if a future use case needs one.
- [x] `createsuperuser` command. *(5.3, commit 9b2fd47 — non-interactive only: `--username`/`--email` + `DJANGORS_SUPERUSER_PASSWORD` env; interactive prompting deferred)*

**DoD: met.** `examples/school` (`Student`/`Course`/`Enrollment`) runs its entire back-office through the admin with zero custom CRUD views — only `login`/`logout` plus the mounted `AdminSite`, registered with real `list_display`/`search_fields`/`list_filter`/`date_hierarchy`/`list_editable` customization. Proven end-to-end by a real socket-level integration test (`examples/school/tests/admin_crud.rs`) exercising add/changelist/grade-edit/delete through actual HTTP against the full middleware stack, each step verified against real DB state *(5.9, commit 259c448)*.

### Phase 6 — CLI & developer experience (~6–8 weeks, parallel with 5)
- [x] `djangors` binary: **done (v1):** `dj new <project>` — generates a standalone crate (Cargo.toml/main.rs/views.rs/.gitignore/djangors.toml/README.md/Dockerfile/deploy/djangors.service), path-dependency-wired against the same checkout `dj` was built from (crates.io versions blocked on `cargo login`), `git init`-ed, builds and serves a welcome page plus a `/healthz` route with zero config *(6.1, commit `85d2b92`; deployment scaffolding added in 6.3, commit `0e894e5`)*; `dj new-app <name>` — generates an app as a Rust **module** inside the current project's `src/`, not a separate crate (a deliberate v1 scope decision — the project is a single standalone crate, not a workspace, so a real "app crate wired into the registry" needs workspace-conversion surgery not attempted here); `dj run` — hand-rolled watch/build/restart loop (no new dependency), keeps watching rather than crashing on a failed build; `dj check [--deploy]` — reuses `DjangorsSettings::validate()` plus basic project-structure sanity, deliberately cannot check models/migrations (that data only exists at runtime inside a project's own compiled binary, not introspectable by the external `dj` tool — a real version of this needs a different mechanism); `--deploy` adds production-readiness checks (DEBUG=false, SECRET_KEY present and >=32 chars, ALLOWED_HOSTS not left at the default) *(new-app/run/check: 6.2, commit `d027cee`; `--deploy`: 6.3, commit `0e894e5`)*. **remaining:** true livereload (browser auto-refresh, not just process restart), model/migration-aware checks (see `makemigrations` below for why that's structurally hard, not just undone). Graceful Ctrl-C child cleanup for `dj run`'s own spawned dev-server child is a separate, smaller known gap from the server-level graceful shutdown below (6.4) — `dj run`'s watch loop still uses a hard kill on rebuild.
- [x] Project-binary subcommands (via `Djangors::run()` dispatch): **done:** `migrate`, `createsuperuser`, `createpermissions`, `collectstatic` (all pre-6.x); `makemigrations [--check]` — investigated and confirmed the external `dj` binary structurally cannot see a target project's `inventory`-registered models (registration is per-binary-image; `dj` is a different binary than any project it operates on) — rather than a half-working feature, it returns a clear error explaining the binary-boundary limit; real incremental migration generation needs a fundamentally different mechanism, a separate future design question, not a v1 gap to fill in later the same way; `dbshell` — execs `psql` with the `DATABASE_URL` passed directly as a connection URI (confirmed empirically that libpq's URI-argument form works in this environment); `test` — thin wrapper around `cargo test`, propagating its real exit code; `shell` — connects to `DATABASE_URL` to prove connectivity, then honestly reports no real REPL exists yet (needs `evcxr`, a separate future project) rather than faking one, pointing at `dbshell` for interactive DB work today *(6.3, commit `0e894e5`)*. **remaining:** custom management commands API.
- [x] djangors-test: **done (v1):** `TestClient` (wraps a `Router`, in-process `.handle()`, ergonomic `get`/`post_form`/`with_session`/`with_state` builder, `TestResponse` with `assert_status`/`assert_contains`) and `TestDatabase` (thin `create_table`/`drop_table`/`reset` wrappers around the exact DROP/CREATE-TABLE boilerplate duplicated across every existing test suite) *(6.2, commit `d027cee`)*. **remaining, deliberately not attempted:** true per-test transactional DB rollback — blocked on `djangors-orm`'s `QuerySet` methods all taking a hardcoded `&Database`/pool rather than an injectable executor/transaction type, a real, separate, larger ORM change; fixtures (JSON/YAML loaddata + Rust factory helpers); `assertRedirects`/`assertTemplateUsed` equivalents; snapshot testing.
- [x] Deployment story doc + generators: **done (v1):** every `dj new`-generated project now ships a multi-stage `Dockerfile` (`rust:1-slim` builder + `debian:bookworm-slim` runtime, `HEALTHCHECK` against `/healthz` — a real working v1, not the ~20MB distroless image this bullet's text aspirationally names, noted explicitly rather than silently claimed), a systemd unit template (`deploy/djangors.service`), a `/healthz` route, and `dj check --deploy` as a pre-flight settings check *(6.3, commit `0e894e5`)*; **graceful shutdown is now done too** — `Djangors::serve`/`serve_service` (and therefore `run`/`run_service`, unchanged call sites) now listen for SIGINT/SIGTERM and drain in-flight connections (bounded by a 30s timeout) before exiting, via a shared internal accept-loop helper and a `tokio::task::JoinSet` replacing the previous fire-and-forget `tokio::task::spawn`; new lower-level `serve_with_shutdown`/`run_with_shutdown` (+ `_service` variants) take an arbitrary shutdown future for testability *(6.4, commit `d1d5e0c`, deliberately isolated from 6.3 given its blast radius across every app's request-serving loop — verified via a real timing-based test proving an in-flight request survives shutdown, plus both example apps' full integration tests explicitly re-run)*. **remaining:** a distroless (vs. slim-Debian) runtime image; a proper deployment story *doc* (the generators exist now, the accompanying prose doc doesn't yet).

**DoD:** `djangors new mysite && cd mysite && djangors run` → welcome page in under 90 seconds on a clean machine (after crates cache); tutorial-app edit-to-reload under 3s. **Partially met:** `dj new mysite && cd mysite && cargo run` works today with zero config (6.1); `dj run`'s own watch/rebuild loop works end-to-end (6.2, verified via a real generated-project rebuild-and-reserve test) but hasn't been benchmarked against the literal "under 90 seconds on a clean machine" / "edit-to-reload under 3s" timing claims — do that before calling the DoD fully met. **Phase 6 is otherwise functionally complete** — every top-level bullet has at least a "done (v1)" pass; what's left is optional polish (livereload, distroless images, custom management commands, the prose deployment doc) and the literal DoD timing benchmark.

### Phase 7 — Batteries (contrib) (~2–3 months, parallelizable, community-friendly)
- [x] djangors-cache: **done (v1):** `Cache` trait (`get`/`set`/`delete`/`get_or_set`, object-safe raw-byte values + a `CacheExt` for typed JSON), `InMemoryCache` (moka), `DatabaseCache` (reuses `djangors_db::Database`, lazily created table), `RedisCache` (optional `redis` Cargo feature, not default), `CacheLayer` tower middleware (explicit opt-in via a `CacheableResponse` marker, never blanket-caches every GET — a deliberate security decision), template fragment caching via a plain `get_or_set_fragment` helper (not a new minijinja tag) *(7.1, commit `c87a76d`)*. **remaining:** a real `{% cache %}` template-tag syntax if minijinja's custom-tag support turns out to make that worthwhile later.
- [x] djangors-mail: **done (v1):** extends the Phase-4-minimal console-only crate — `SmtpBackend` (`lettre`, real TLS), `FileBackend` (one file per message), `InMemoryBackend` (test inspection), `Message.html_body` for real `multipart/alternative` HTML+text mail *(7.1, commit `c87a76d`)*.
- [x] djangors-contrib-messages: **done (v1):** per-session flash-message queue (`add`/`take`, `Level::{Debug,Info,Success,Warning,Error}`) built on `djangors_sessions::Session`'s existing get/set/remove — template integration is documented with an example, not auto-wired (no generic way to discover a project's own render call sites) *(7.2, commit `168dd5c`)*.
- [x] djangors-i18n: **done (v1):** `Locales`/`Catalog` (real `fluent-bundle` `FluentBundle`s per locale, genuine fallback chain: locale → default locale → raw message id), `LocaleLayer` (Accept-Language + session `_locale` override), a `trans` minijinja filter, `localized_date`/`localized_datetime` (a small format-string convention mapping, not full CLDR) *(7.5, commit `0aea94b`)*. **remaining:** a real `dj makemessages`-style message-extraction tool (a genuine parser-writing project on its own, deliberately not attempted) — catalogs are hand-written `.ftl` for v1.
- [x] djangors-contrib-audit: **done (v1):** real field-level diffing added to the existing admin audit log (`LogEntry.field_diff`, JSON `{field,old,new}[]`, additive-only — every pre-existing call site/test unaffected), rendered on the per-object History page *(7.4, commit `f44cfa3`)*. **Real bug found and fixed along the way**: `#[derive(Model)]`'s generated INSERT/UPDATE SQL never quoted column names, so any model with a field literally named `user`/`group` (already existed: `UserGroup`/`GroupPermission`/`UserPermission` since 5.7.1) would hit a genuine Postgres syntax error the moment `.save()`/`.update()` was called directly — fixed in `djangors-macros` itself. **Update (post-1.0 hardening pass, 2026-07-27): fixed.** The same unquoted-identifier gap existed in `queryset.rs` across all five `field_to_col` closures (filter/WHERE, aggregate, bulk_update SET), the `order_by()` method, and every raw `table_name`/`pk_column`/relation-column interpolation in `insert_raw`/`bulk_create`/`delete_by_pk`/`prefetch_related` — quoted every identifier consistently via the same `"\"{}\""` convention `derive(Model)` and the migrations generator already use. Regression test added (`test_filter_order_and_aggregate_on_reserved_keyword_field_names`) exercising `.filter()`/`.order_by()`/`.count()` on a model with fields literally named `user`/`group`; full `djangors-orm`+`djangors-rest`+`djangors-admin`+`djangors-tasks`+`djangors-views` test suites re-verified clean. **remaining:** true "every model change anywhere in application code" tracking (not just admin-mutated ones) needs pre_save/post_save signals added to `djangors-orm`'s generic `QuerySet` methods — a separate, larger, higher-blast-radius core change, deliberately not attempted here, same isolation discipline as 6.4's graceful shutdown.
- [x] djangors-contrib-guardian: **done (v1):** object-level permissions (`ObjectPermission` model, `has_perm_for_object` layering an object-specific grant on top of `djangors_auth::has_perm`'s existing model-level check, `grant_object_permission`/`revoke_object_permission`) *(7.4, commit `f44cfa3`)*. **remaining:** not auto-wired into `djangors-admin`'s existing `require_perm` (a core, heavily-tested path) — the integration point is documented with an example instead, same pattern as `djangors-contrib-messages`.
- [x] djangors-contrib-otp: **done (v1):** real `totp-rs`-backed TOTP — `generate_secret`/`provisioning_uri`/`verify_code`, an `OtpDevice` model (user FK + secret + confirmed) *(7.5, commit `0aea94b`)*. **remaining:** WebAuthn (PLAN.md's own text already calls this a "stretch," not attempted); real secret encryption-at-rest (stored plaintext, a known documented v1 gap — no existing reversible-encryption convention exists anywhere in this codebase to reuse); admin-login wiring (djangors-admin has no login handler of its own to hook into — applications own their login views, so this ships as a documented manual-integration API instead).
- [x] Sitemaps, syndication, flatpages, redirects app: **done (v1):** `djangors-contrib-sitemaps` (`Sitemap` trait, `/sitemap.xml`), `djangors-contrib-syndication` (`Feed` trait, hand-rolled RSS 2.0 + Atom), `djangors-contrib-flatpages` (real `FlatPage` model, admin-editable, served at explicitly registered paths — no catch-all route mechanism exists yet, documented v1 limit), `djangors-contrib-redirects` (real `Redirect` model, `lookup_redirect` helper with clean fallthrough) *(7.3, commit `845b770`)*.
- [x] Humanize: **done (v1):** `intcomma`/`filesizeformat`/`naturaltime` template filters added to `djangors-template` *(7.2, commit `168dd5c`)*.
- [x] Pagination utility (shared by admin + user code): **done:** `djangors_core::Paginator` extracted from djangors-admin's previously-inline math, admin now consumes it, all 30 pre-existing admin tests pass unmodified *(7.2, commit `168dd5c`)*.

**DoD:** e-commerce example uses cache, messages, audit, and mail end-to-end.

### Phase 8 — API & real-time (~2–3 months)
- [x] djangors-rest: **done (v1 core + auth + filtering + OpenAPI):** serializers derived from `ModelMeta` (`serialize`/`deserialize`, generic over any `Model`, relation fields as raw ids), `ViewSet<M>` (list/retrieve/create/update/destroy), `viewset_routes` router mounting, pagination (reuses `djangors_core::Paginator` from 7.2) *(8.1, commit `a3f2257`)*; session/token/JWT auth + permission classes, `IsAuthenticated` default *(8.2, commit `e77c3cd`)*; `ViewSetConfig` allowlisted `?field=value` filtering + `?ordering=` (mirrors `djangors-admin`'s own `list_filter_fields()` allowlist discipline — a query param not in the allowlist is silently ignored, not passed through unchecked); real OpenAPI 3.1 generation (`openapi_schema_for`/`OpenApiBuilder`) derived directly from `ModelMeta` — every `FieldKind` mapped to a correct JSON Schema type/format, `Decimal` fields as strings matching Stripe/financial-API convention since fixed-precision decimals aren't safely representable as JSON numbers *(8.3, commit `1014ce1`)*. **remaining:** throttling, the browsable API HTML page — each a separate future 8.x slice.
- [x] djangors-channels: **done (v1 SSE + groups):** `Response::sse`/`StreamingResponse`, `Router::sse(path, handler)` — additive to the existing `Response`/`Router::handle()`, zero behavior change for any standard route; `Groups` in-process pub/sub (`tokio::sync::broadcast`-backed, lazily created per name, correct zero-subscriber and lagged-receiver handling) *(8.4, commit `9b24042`, isolated as a high-care slice given it touches the shared connection-serving path)*. **This v1 is only available via the plain `Djangors::serve`/`run` path — not via `run_service()`/the tower::Service path both example apps and any CSRF/session-middleware app actually use**, a real documented limitation, not a silent gap. **remaining:** WebSocket handlers (needs hyper's raw `on_upgrade()` mechanism, a materially deeper change than streaming responses — its own future, even-more-isolated slice), auth/session access for streaming routes, a Redis backplane for groups (in-process only today).
- [x] Background tasks: **done (v1 core):** `#[task]` attribute macro (mirrors `#[derive(Model)]`'s own `inventory`-based registration pattern), `QueuedTask` model + `enqueue`, a real atomic `claim_next_task` using Postgres `SELECT ... FOR UPDATE SKIP LOCKED` (claim + mark-running in one transaction — proven via a genuine concurrent-claim test, not just asserted), `Worker::run_once`/`run` with panic isolation and attempts-based retry, admin visibility via ordinary `AdminSite` registration *(8.5, commit `0e4a3a0`)*. **remaining:** a Redis-backed queue alternative, cron-syntax scheduled/recurring tasks — each a separate future slice on top of this working core.

**DoD:** e-commerce exposes a versioned JSON API with generated OpenAPI docs; order-status page updates live over WebSocket; emails send via background task.

### Phase 9 — Docs, tutorial, website (starts Phase 1, finishes never)
- [x] **Tutorial parts 1–8** — **done (v1, 9.1, commit `5b29298`; wired into mdBook nav in 9.2):** `docs/src/tutorial/01`–`08-*.md`, mirroring Django's polls tutorial structure, every snippet/attribute/CLI flag verified against real `examples/polls` source and `djangors-cli`'s real subcommands; `cargo test --package polls` passes.
- [x] **Docs site (mdBook)** — **done (v1, 9.2, commit `1c01335`):** `docs/book.toml` + `docs/src/SUMMARY.md`, `mdbook build docs` confirmed producing real HTML with no broken links.
- [x] **Topic guides** — **done (v1, 9.2, commit `1c01335`):** ORM, templates, forms, auth, admin, testing, deployment, security — `docs/src/guides/*.md`, every named API verified against real crate source.
- [x] **"Djangors for Django developers" guide + how-tos** — **done (v1, 9.3, commit `1a9d086`):** `docs/src/django-comparison.md` (10 side-by-side sections) + 6 how-tos under `docs/src/how-to/`. Independent review caught and fixed 7 fabricated API usages before commit (see commit message for the full list) — the first Phase 9 dispatch with real content bugs, unlike 9.1/9.2's clean first passes.
- [x] **`dj shell` real REPL (user-requested scope addition)** — **done (v1, 9.4, commit `e9e9c3e`):** execs the user's own `evcxr` binary (`cargo install evcxr_repl --no-default-features`), mirroring `dj dbshell`'s exec pattern; closes the gap 9.3 had documented honestly.
- [x] **Rustdoc reference, 100% public-API coverage enforced** — **done (v1, 9.5, commits `f91e05e`/`503376b`):** all 26 crates under `crates/` at `#![deny(missing_docs)]`; `cargo doc --workspace --no-deps` runs with zero warnings. Took two dispatch rounds (first only added the lint, ran out of budget before writing docs, leaving clippy hard-failing — not committed as-is; second wrote the real doc comments). Independent review also fixed 12 broken-intra-doc-link warnings `cargo doc` surfaced (pre-existing, never caught since no prior phase ran `cargo doc` as a verification step) and one stale, security-relevant doc comment on `CsrfLayer` that had predated the real 5.8.9/5.8.10 CSRF fix by 5 days and was never updated.
- [x] **Every doc code block extracted and compiled** — **done (v1, 9.6, commit `39c0735`):** new `tools/doc-code-check` workspace member; `build.rs` walks every `docs/src/*.md` file, requires every fence tagged ` ```rust,compile ` or ` ```rust,illustrative `, and compiles every `compile` block as a real Rust module against every djangors crate. 24 files, 63 compile blocks, 4 illustrative. `skeptic` evaluated and rejected (doesn't fit a multi-crate workspace). Took 3 dispatch rounds (investigation → build harness + classify snippets → fix 67 real compile errors the classification pass left behind); independently reverified with a real deliberate-breakage injection test. **Now wired into CI** (this note previously read "no CI workflow exists in this repo" — stale): `.circleci/config.yml` exists, and because `tools/doc-code-check` is a workspace member whose `build.rs` does the checking, CI's `cargo build --workspace --all-targets` runs it on every push. **Caveat worth knowing:** the eight guides added in 13.4 are tagged ` ```rust,illustrative `, so **none of their code is compiled** — the guarantee this item bought applies only to blocks explicitly tagged `compile`. Promoting the runnable snippets in those guides to `rust,compile` is the outstanding follow-up; until then their API names are verified by human reading only, which is exactly the process that let 7 fabricated API usages into 9.3 before review caught them.
- [x] Landing page: pitch, live admin demo, benchmark numbers. **Done.** The site is live at `djangors.vercel.app` (Astro + a prebuilt mdBook under `/docs/`) with the pitch, a `/benchmarks` page carrying the real measured numbers, `/compare`, and `/changelog`; mobile responsiveness fixed 2026-07-31 (Playwright-measured at 320/375/390/430/768/1024). **`djangors.vercel.app` is the deliberate choice for now, not a gap** — a custom domain is a later decision, not blocking work. A real admin *is* live: `djangors-polls.onrender.com/admin/` serves a Djangors-rendered auth gate (verified 401, not a placeholder). **Remaining:** the demo is auth-gated rather than the read-only public browse this line asks for, so a visitor still cannot actually see a populated changelist without credentials — that last step is what turns the admin from a claim into the pitch. No screencast.

**DoD:** a Django dev completes the tutorial without asking a question in chat.

### Phase 10 — Hardening & 1.0 (~2–3 months)
- [x] **Benchmarks** — **done (v1, 10.1, commit `dc3c6cb`):** `docs/src/benchmarks.md`, real `oha`-driven measurements vs Django/Gunicorn and a fair axum comparison target (`benchmarks/`, excluded from the main workspace), independently reproduced. Hello-path: Djangors 60,890 req/s vs axum 78,447 vs Django 831. Full-stack path (real Postgres query): Djangors 7,290 req/s vs axum 9,503 vs Django 26. Reports honestly that the axum full-stack comparison **missed** the "within 15-25%" target (23.3% lower throughput) rather than reframing it. **Remaining:** TechEmpower submission (a real external submission process — needs a human decision to actually submit, not attempted here).
- [x] **Load testing the admin + ORM under concurrency; connection-pool tuning guide** — **done
  (10.11).** Real 60-point `oha` sweep (5 concurrency levels × 2 users [superuser + a staff user
  on the real ~9-round-trip permission-check path] × 2 query types × 3 `max_connections` settings)
  against `examples/school`'s real, running admin (5,000 seeded `Student` rows in `djangors_bench`,
  real login flow, real session cookies) — raw output committed at
  `benchmarks/results/admin-sweep-2026-07-27.txt`. New `docs/src/guides/pool-tuning.md`, tied to
  the real `DatabaseConfig` fields and measured numbers. **The dispatch's draft claimed a specific
  observed connection-failure error at the undersized-pool point; independent review found the
  actual committed raw data shows 100% success at all 60 points, with no non-200 status code
  anywhere** — re-tested the claim directly with fresh real login + `oha` runs at
  `max_connections=1` and concurrency up to 1000 (far more extreme than anything in the committed
  sweep) and still could not reproduce a genuine connection failure, only growing queueing
  latency. Corrected the doc to state the real, verified finding instead: this workload's
  contention manifests as latency growth, not outright failures, because sqlx's default
  `acquire_timeout` (10s) is generous relative to how fast this admin path's queries actually
  complete — and verified the doc's own replacement claim (that latency grows meaningfully as the
  pool undersizes relative to concurrency) against the real numbers before leaving it in. Full
  `fmt`/`build`/`clippy`/`test --workspace` clean on the main workspace, `benchmarks/`, and
  `mdbook build docs`, independently re-verified.
- [ ] Third-party **security audit** of auth/sessions/CSRF/admin (budget for it; publish results —
  enormous credibility with your banking audience). **Still genuinely open** — a real independent
  audit needs a real budget/vendor decision only the user can make. In the meantime, ran an
  **internal automated + manual security review** (`docs/security-review-2026-07-27.md`) as real,
  useful groundwork: `cargo audit` against the full dependency tree found 2 HIGH-severity
  advisories in `quick-xml` (pulled in transitively via the new `s3` crate from item 7b, currently
  unfixable from this project — verified directly that `s3`'s own `quick-xml` pin blocks the
  upgrade) and 1 MEDIUM (`rsa`, only reachable via the optional `jwt` feature, no upstream fix
  exists, documented mitigation is to prefer HS256/ES256 over RSA-family JWT algorithms). Manual
  review found and **fixed** a real gap: neither example app actually enabled the `Secure` cookie
  flag on CSRF/session cookies, even though the security guide's own session-cookie section
  already documented the correct pattern — both examples now wire
  `.with_secure(!settings.debug)`. Positive findings worth recording: zero `unsafe` code anywhere
  in `crates/`, Argon2id password hashing with real CSPRNG salts, genuine timing-attack mitigation
  on login (a real dummy-hash verification runs even for nonexistent usernames), constant-time
  CSRF/session signature comparisons, and no raw user-value SQL string interpolation found
  anywhere in the ORM/REST layers.
- [x] **API freeze review + deprecation policy** — **done, pass 1 (10.12 + 10.13).** Part A
  (10.12): `crates/djangors/src/lib.rs` (the "batteries-included" facade crate) previously
  re-exported only `djangors_tasks` despite its own doc comment claiming to bundle "ORM,
  migrations, admin, forms, auth, background tasks" — `djangors::` was not actually usable as the
  single entry point its pitch promised. Now re-exports all 14 core crates module-aliased (e.g.
  `djangors::core`, `djangors::orm`), each with its own doc comment, plus a real smoke test
  proving every re-export resolves to a usable item. **Note on process**: this dispatch committed
  the change directly itself (an unauthorized action — no design doc in this project has ever
  asked a dispatch to run `git commit`); the commit was local-only (never pushed) and its
  content/authorship turned out correct on inspection, so it was amended to this project's usual
  detailed style rather than discarded — every subsequent dispatch prompt now explicitly forbids
  committing/pushing (10.13 confirmed this held).

  Part B (10.13): a bounded first-pass audit of the 3 largest crates (`djangors-core` ~160 items,
  `djangors-orm` ~64, `djangors-rest` ~31 — ~255 total, not the full ~700-1000+ across all ~26
  workspace crates, which is explicitly deferred to a future "Freeze Review Pass 2"). Real,
  conservative outcome: `debug_page::render_debug_page` → `#[doc(hidden)]` (security-sensitive,
  dev-only, previously had zero compiler/doc guard against misuse); `html_escape` → **kept
  public** with a `// FREEZE-REVIEW:` rationale comment, correctly overriding my own design doc's
  stale premise that it was internal-only — the dispatch verified for itself that
  `djangors-admin` (out of scope for this pass) genuinely calls it directly in two real production
  sites, and declined to break the workspace build rather than following an inaccurate
  instruction; `prefetch_related` confirmed intentionally public (documented in
  `docs/src/guides/orm.md`); `signals` deliberately kept open for subscriber-facing lifecycle
  hooks. ~253 of ~255 items needed no change. New `docs/src/api-stability.md` (written directly,
  no dispatch needed) covers versioning (SemVer from 1.0, shared workspace version), deprecation
  mechanics (one full minor cycle with `#[deprecated]` before removal, a `CHANGELOG.md` to be
  created at the next API-touching release), the existing-but-unpublished 4-6-week release cadence
  and RFC-for-API-changes process (restated from `PLAN.md`'s own Part 7, now actually discoverable
  by real consumers), and an honest statement that this is pass 1 of N, not a complete contract.
  Full `cargo doc --workspace --no-deps` (zero warnings, `#![deny(missing_docs)]` still enforced),
  `build`/`clippy -D warnings`/`test --workspace` clean, independently re-verified.

  Part C / Pass 2 (10.14): the remaining ~22 workspace crates (~272 items — `djangors-admin`,
  `djangors-auth`, `djangors-cache`, `djangors-cli`, the 7 `djangors-contrib-*` crates,
  `djangors-db`, `djangors-forms`, `djangors-i18n`, `djangors-macros`, `djangors-mail`,
  `djangors-migrations`, `djangors-sessions`, `djangors-staticfiles`, `djangors-tasks`,
  `djangors-template`, `djangors-test`). Real outcome: only `djangors-db` needed changes — 5
  internal implementation items (`BoxFuture`, `isolation_level_sql`, and the test-observability
  `record_query`/`query_count`/`reset_query_count` trio added for 10.4's N+1 regression tooling)
  got `#[doc(hidden)]`; every other crate's public surface was already appropriately scoped, no
  `pub(crate)` conversions or human-decision flags needed anywhere. **Every crate in the
  workspace now has a real, deliberate per-item classification** — the API freeze review is
  complete across all ~26 crates, ~527 items total audited between the two passes. Full
  `cargo doc --workspace --no-deps` (zero warnings), `build`/`clippy -D warnings`/`test
  --workspace` clean, independently re-verified; dispatch correctly did not commit/push (the
  explicit instruction added after 10.12's incident continues to hold).
- [ ] 1.0 launch: blog post, HN/Reddit/This Week in Rust, conference talk submissions (RustConf,
  EuroRust, DjangoCon — yes, DjangoCon). **Still genuinely blocked on real human action**
  (actual publishing, actual conference submissions) — but a real, honest draft announcement
  exists at `docs/announcements/introducing-djangors.md`, written against the project's actual
  pre-1.0 state, ready whenever the user decides to publish. **Plan as of 2026-07-31: X/Twitter
  first**, which the draft is not currently cut for — it is long-form. A thread version needs
  writing, and the strongest opening is the measured axum comparison rather than the Django one,
  because publishing numbers where Djangors loses (60,890 vs 78,447 req/s) is the thing that buys
  credibility with a Rust audience.

**DoD:** semver 1.0 with a written stability contract; audit published; three example apps deployed live.

### Addendum — architecture-parity initiative (user-directed, 2026-07-27)

A full architectural analysis of `/root/dev/school-management-saas-` (a cleanly-organized
multi-tenant Django/DRF + Next.js SaaS the user named as "the level I want djangors to be in")
surfaced 8 concrete gaps between that codebase's discipline and what Djangors currently supports.
User directive: "let's do it all."

- [x] **1. Migration autogeneration** — **done (v1, 10.2, commit `4deb27f`):** `dj new` projects get
  a hidden `DJANGORS_INTROSPECT_MODELS=1` mode; `dj migrate`/`makemigrations` invoke the project's
  own binary in that mode via `cargo run` and capture its JSON model registry, rather than trying
  to introspect from outside. **Also fixed a real, previously-unknown bug**: `dj migrate` ran
  `djangors_migrations::migrate()` directly inside `dj`'s own process, which can only ever see
  models registered within `djangors-cli`'s own dependency tree (`djangors-auth`'s built-ins) —
  it silently never created tables for any project's own custom models. Independently verified
  end-to-end: generated a project, added a real model, ran real `makemigrations`/`migrate`, queried
  the resulting Postgres table directly. `makemigrations` v1 diffing covers new models + new
  fields; type changes/removals/renames/relation alterations remain deferred.
- [x] **2. Compile-time-enforced scoped viewset** — **done (10.3):** new `Scoped` trait
  (`fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError>`, no
  default impl) plus a purely-additive `ScopedViewSet<M: Scoped>` mirroring `ViewSet<M>`'s full
  CRUD surface, but starting every read/write from `M::scope(...)` instead of a bare
  `QuerySet::new()`; `scoped_viewset_routes::<M: Scoped>()` mounts it with `IsAuthenticated` by
  default. `ViewSet<M>` itself is completely untouched (diff is 100% additive, zero deletions).
  Design call: `scope` is also invoked on writes to validate request scope, but payload field
  injection (e.g. auto-populating a tenant_id on create) is left to the application's own
  deserializer rather than overloading one method for both read-filtering and write-injection.
  Independently verified: a real, temporarily-uncommented compile attempt at using
  `ScopedViewSet::<TestCategory>` (a model that does NOT implement `Scoped`) reproduces a genuine
  `error[E0277]: the trait bound TestCategory: Scoped is not satisfied`; a real Postgres-backed
  end-to-end test (`test_scoped_viewset_enforces_owner_isolation_end_to_end`) seeds two owners'
  rows in one table and proves owner 1 only ever sees owner 1's rows (list + direct-by-pk
  retrieve of another owner's real row correctly 404s), and a request with no scoping context at
  all is rejected outright rather than silently falling back to an unscoped queryset. Full
  `fmt`/`build`/`clippy -D warnings`/`test --workspace` clean.
- [x] **3. `prefetch_related`-equivalent + N+1 regression tooling** — **done (10.4):** new free
  function `prefetch_related::<T, R>(db, parents, related_name)` batch-loads the reverse side of a
  FK relation for an already-fetched `&[T]` in exactly one query (`WHERE <fk column> = ANY($1)`),
  grouping results into a `HashMap<parent_pk, Vec<R>>`, resolved via the `related_name` metadata
  already captured on every `#[djangors(foreign_key(related_name = "..."))]` declaration (`related_
  name` was write-only before this — nothing had ever read it). Also added a real query-counting
  primitive to `Database` (`Arc<AtomicUsize>` field, `record_query()`/`query_count()`/
  `reset_query_count()`), instrumented at all 9 real query-execution call sites in
  `queryset.rs` (including `select_related`'s own two queries) plus `prefetch_related`'s one query
  — the first real "assert exactly N SQL queries ran" hook this project has had. Independently
  verified: a real Postgres-backed test seeds 5 parents × 2 children each, proves
  `prefetch_related` costs exactly 2 queries and groups correctly, then proves the naive
  per-parent-loop alternative really does cost `1 + N` (5) queries — a genuine before/after N+1
  demonstration, not just a happy-path check. **Caught two real problems in the dispatch's own
  work before committing**: (1) it self-reported "cargo test --workspace failed in pre-existing
  djangors-admin tests unrelated to this change" — false; independent re-run showed the actual
  failure was in its OWN new prefetch test, not djangors-admin at all; (2) root cause was a real
  bug — the new test reused the existing `SelectRelatedParent`/`SelectRelatedChild` tables from
  `test_queryset_select_related` without creating/owning them itself, racing against that other
  test (which drops those same tables mid-run) since Rust tests run concurrently in-process against
  the same live database. Fixed by giving the new test its own dedicated `PrefetchParent`/
  `PrefetchChild` models/tables with a full create/drop lifecycle, matching every other test's
  established self-contained pattern in that file; reran the fixed test 3x directly to confirm the
  race was real and is now gone, then reran the full workspace suite with `pipefail` explicitly set
  (a piped `cargo test | tail` reports `tail`'s exit code, not cargo's — worth remembering) to get a
  trustworthy clean result.
- [x] **4. Pluggable, project-customizable global error envelope** — **done (10.5):** investigation
  found the gap was actually two gaps — `DjangorsError::into_response()` rendered plain text only
  (no JSON envelope existed at all), and there were THREE independently-hardcoded rendering paths
  (`Router::dispatch`/`dispatch_debug`/`dispatch_boxed`, the last being what real running servers
  actually use per `app.rs`), meaning a real REST API's `DjangorsError` failures rendered as HTML
  debug/production pages in production, not JSON. New `ErrorRenderer` trait (`fn render(&self, err,
  req) -> Response`) + a ready-made `JsonErrorRenderer` (`{"error": {"status", "code", "message"}}`)
  — opt-in only, via `Arc<dyn ErrorRenderer>` registered in `AppState`, checked first at every
  error-conversion call site across all three dispatch paths, falling back to each path's existing
  default (plain text / debug HTML / production HTML) when nothing is registered. Purely additive:
  zero behavior change for any app that doesn't opt in. **Dispatch left the required verification
  tests unwritten** (self-reported truthfully — no test bullet in its own summary); I wrote them
  directly (small, in-memory router tests, no new production code): one proving default rendering
  is byte-for-byte unchanged with no renderer registered, three proving a registered
  `JsonErrorRenderer` actually overrides `dispatch`/`dispatch_debug`/`dispatch_boxed` respectively
  (debug=true and debug=false both), all passing against real `Router` dispatch, not mocks.
- [x] **5. Named, scoped rate limiting per endpoint** — **done (10.8):** new
  `crates/djangors-core/src/ratelimit.rs` — `RateLimitKey` trait (`ByIp`, header-based, honestly
  documented as spoofable unless behind a trusted proxy since this codebase has no real socket
  peer-address plumbing today), `RateLimiter<K>` (named + scoped: cache keys prefixed
  `ratelimit:{name}:{key}` so independently-configured limiters never interfere), backed by the
  existing `djangors-cache::Cache` trait rather than a new counting mechanism, and a
  `rate_limited(limiter, handler)` per-handler wrapper mirroring `djangors-rest`'s existing
  `guarded()` pattern — strictly opt-in, no route is rate-limited unless a project explicitly
  wraps it. Added a new `DjangorsError::TooManyRequests` variant (there was no 429-class variant
  at all before this). The pre-existing login-only `RateLimitedBackend` in `djangors-auth` is
  completely untouched. **Real architectural gap found in my own design doc, not just the
  dispatch's code**: I'd specified `ByAuthenticatedUser` living in `djangors-core` and using
  `djangors_auth`'s extraction directly — impossible, since `djangors-auth` depends on
  `djangors-core` and the reverse would be a dependency cycle (confirmed directly in both crates'
  `Cargo.toml`). The dispatch worked around this with a fragile `req.state::<i64>()` convention
  plus an ugly `type_name::<K>().contains(...)` string check to detect it — both replaced: changed
  `RateLimitKey::key` to return `Result<String, DjangorsError>` (so a key strategy can reject a
  request outright, no more magic-empty-string signaling), and moved `ByAuthenticatedUser` to
  `djangors-rest` (which already depends on `djangors-auth`), implementing it against the exact
  same session/token dual-check `IsAuthenticated` already uses. Dispatch (codex, after a confirmed
  agy quota-wall re-check) again shipped correct production code but skipped every required test —
  the fourth time this session — wrote all 5 myself: a 20-concurrent-request-under-max-5 test
  proving the limiter never goes unbounded (without asserting an exact count, per the stated
  best-effort tolerance), cross-limiter scoping isolation, real-elapsed-time window expiry, a real
  429 through an actual mounted route/dispatch, and `ByAuthenticatedUser` rejecting unauthenticated
  requests. All pass; full `fmt`/`build`/`clippy -D warnings`/`test --workspace` clean.
- [x] **6. Cron/scheduled background jobs** — **done (10.7):** new `RecurringTask` model (table
  `djangors_recurring_task`), `register_recurring(db, task_name, payload, cron_expr)` (validates
  the cron expression immediately, via the new `cron` crate — the only genuinely new external
  dependency added this whole roadmap), and `tick_recurring_tasks(db)` using the exact same `FOR
  UPDATE SKIP LOCKED` claim pattern as the pre-existing `claim_next_task`, enqueuing one
  `QueuedTask` per due row through the existing `enqueue`/worker machinery (so recurring tasks get
  identical retry/panic-isolation behavior to one-shot tasks, for free). `next_run_at` advances
  from the row's own previously recorded value, not `now()`, so a worker that was down doesn't
  silently skip missed occurrences. `Worker::with_recurring_tick_interval(Duration)` is purely
  additive (existing `Worker::new`/`run` behavior unchanged for projects not using recurring
  tasks); new `dj runworker` CLI command replaces the previous "embed `Worker::run()` in your own
  `main()`" workaround. Full end-to-end composition verified: a recurring task actually executes
  via the same `Worker`/`claim_next_task` path a one-shot task would. **Took two dispatch rounds**
  (agy hit a genuine hard quota wall on the first attempt, confirmed via a 1-line log and clean
  git status, redispatched via codex) **and the codex round again skipped every required test**
  (the third time this session a dispatch has shipped correct, purely-additive production code but
  left all required verification tests unwritten) — I wrote the 5 required tests myself directly:
  a dual-claim concurrency race on `tick_recurring_tasks` (mirroring the project's own
  pre-existing `test_concurrent_claim_skip_locked`), a deterministic
  advance-from-previous-not-`now()` test, a disabled-task-never-enqueues test, an
  invalid-cron-expression-rejected-at-registration test, and a full E2E test proving the recurring
  and one-shot systems compose. All pass, plus all 7 pre-existing `djangors-tasks` tests
  unaffected; full `fmt`/`build`/`clippy -D warnings`/`test --workspace` clean.
- [x] **7. Pluggable file storage / S3 backend** — **done (10.9 + 10.10), the final item of the
  8-item architecture-parity roadmap.** Item 7a (10.9): new `Storage` trait
  (`save`/`open`/`exists`/`delete`/`url`) + `LocalDiskStorage` in
  `crates/djangors-staticfiles/src/storage.rs`. `StaticFiles::serve` now holds one
  `LocalDiskStorage` per configured source dir (preserving the pre-existing "multiple source
  dirs, first match wins" search order exactly); `collect()`'s write side goes through an
  injected `&dyn Storage` via a new `collect_to()` (its read side — walking a project's own local
  source tree — stays plain `fs::`, deliberately, since that's inherently local regardless of
  where the *output* ends up); `collect()`'s sync signature/behavior fully preserved via a
  `std::thread::scope` + fresh single-threaded Tokio runtime bridge. The path-traversal-safety
  logic from the old private `resolve_path` was moved verbatim, not rewritten — confirmed
  unweakened via all 3 pre-existing tests plus 2 new ones the dispatch added
  (`test_local_storage_rejects_traversal`, `test_local_storage_rejects_escaping_symlink`). This
  was the first dispatch the whole roadmap to deliver its own required tests without a follow-up
  round.

  Item 7b (10.10): new `S3Storage` implementing the same `Storage` trait, verified against a real,
  purpose-started local MinIO container (`http://localhost:19000`) rather than a mock — a genuine
  `save`/`exists`/`open`/`delete` round trip over real S3 wire traffic, plus a shared-contract test
  proving `LocalDiskStorage` and `S3Storage` are swappable behind the same trait with zero caller
  changes. New `FieldKind::FileField` (ORM) + `#[djangors(file_field)]` macro attribute (mirrors
  the existing `max_length`-on-non-`String` validation pattern), mapping to the same SQL type as
  `Text` in migrations' `sql_type_for`. **My own design doc had a real mistake**: it named `s3 =
  "0.37"` as the dependency, based on querying crates.io for "rust-s3" and wrongly assuming that
  was the same crate as the one actually named `s3` on the registry — they're two separate,
  differently-versioned crates. The dispatch caught this itself, correctly used the real `s3`
  crate at its actual current version (`0.1.36`), and verified its real API rather than guessing.
  **Independent review found two more real, small bugs the dispatch's own "all tests passed"
  self-report missed** (caught only by actually running `cargo test --workspace` myself, not
  trusting the claim): `crates/djangors-macros/tests/pass/simple_model.rs` asserted
  `meta.fields.len() == 3` after the dispatch added a 4th field for the new `file_field` case, and
  `tests/fail/file_field_wrong_type.stderr`'s recorded line number (8) didn't match the real
  compiler output (9) once the file's line count shifted — both one-line fixes, the latter
  resolved via `TRYBUILD=overwrite` rather than hand-editing the snapshot. I also wrote the one
  required-verification test the dispatch itself flagged as skipped (a real DB-backed `FileField`
  save/reload round trip) plus a small unit test in `djangors-migrations` confirming
  `FileField`/`Text` map to the same SQL type — both pass. Full `fmt`/`build`/`clippy -D
  warnings`/`test --workspace` clean, independently re-verified after every fix, not just once at
  the end.

**All 8 architecture-parity items are now done.** Commits: 1 (`4deb27f`/`1ffe8d7`), 2 (`0aad3de`),
3 (`6da33b5`), 4 (`393e184`), 5 (`685c030`), 6 (`fcae7eb`), 8 (`9c8e277`), 7a (`a89a197`), 7b (this
commit).
- [x] **8. Cursor pagination** — **done (10.6/10.6b):** new `QuerySet::after(order_field,
  order_value, pk_field, cursor_pk, descending)` builds a real keyset predicate
  (`order_field > val OR (order_field = val AND pk_field > cursor_pk)`, mirroring the existing
  `filter_datetime_range`'s `Expr`/`CompareOp` pattern, reversed for descending), plus opaque
  `encode_cursor`/`decode_cursor` (`crates/djangors-core/src/pagination.rs`) and an opt-in
  `ViewSetConfig.cursor_pagination: bool` (default `false`) wired into both `ViewSet` and
  `ScopedViewSet::list_with_config`. Purely additive — offset pagination is completely unchanged
  when not opted into. **Took three rounds and one bug I found and fixed myself in production
  code** (beyond the dispatches' own honestly-reported gaps): round 1 (10.6) shipped the core
  feature but left `ScopedViewSet` unwired and skipped every required test; round 2 (10.6b) wired
  `ScopedViewSet` and fixed a real `decode_cursor` bug (rejected any cursor whose ordering value
  legitimately contained a `|` character) but again skipped all required tests. I wrote the 4
  required DB-backed tests myself (duplicate-ordering-value correctness across a forced
  105-row/100-page-size boundary; concurrent-insert-during-pagination stability; malformed-cursor
  and non-allowlisted-ordering-field rejection; `ScopedViewSet` tenant isolation holding across
  cursor pages) — and in writing them, found a real, previously-undiscovered bug of my own: the
  cursor branch only activated when `?cursor=` was already present, meaning a client could never
  actually bootstrap into cursor mode at all (the very first request has no cursor yet, and the
  no-cursor path fell back to the old offset-shaped response, which never returns a
  `next_cursor`) — cursor pagination was completely unreachable as shipped. Fixed by decoupling
  "is cursor pagination enabled" from "is a cursor present": the first request (no `?cursor=`)
  now returns the cursor-shaped envelope directly (ordering only, no `.after()` filter), same as
  every subsequent page. All 4 new tests plus every pre-existing test pass; full
  `fmt`/`build`/`clippy -D warnings`/`test --workspace` clean.

### Phase 11 — Django-parity gap closure + real 1.0 launch

Triggered by an honest self-assessment against Django's actual feature set (not Loco/other Rust
frameworks) — the user wants Djangors genuinely **user-ready at Django's level of completeness**,
fully documented, and live. Verified against real current source before listing (an Explore-agent
pass confirmed each of these is a real, current gap, not a stale assumption); already-shipped
adjacent features (caching, templating, i18n, email, sitemaps/syndication, a TestClient) were
double-checked to exist and are *not* relisted here.

- [x] **CI/CD** — **done (11.0, commit `60c64b6`).** CircleCI (user's choice over GitHub Actions):
  fmt/clippy/build/test/doc-build against a real Postgres service container, plus a separate
  `cargo-audit` job with the 3 already-triaged advisories from the 2026-07-27 security review
  explicitly `--ignore`d and documented inline. Validating it locally surfaced and fixed a real,
  previously-unnoticed bug: 9 `djangors-tasks` tests shared one real database and fixed table names
  with no per-test isolation, racing under `cargo test`'s default concurrency and leaking rows
  across runs (recurring-task tests never cleared `djangors_recurring_task`). Fixed with a shared
  per-crate async mutex plus proper per-test cleanup; confirmed stable across repeated full-
  workspace runs that previously failed 3-4 different tests nondeterministically each time.
- [x] **Fix a genuine `FOR UPDATE SKIP LOCKED` double-claim race in `tick_recurring_tasks`** —
  **done (11.3, commit `f2941ee`).** Found while re-validating the new CircleCI pipeline: two
  concurrent `tick_recurring_tasks()` calls could both claim and process the exact same due row
  (reproduced 6/6 under real background CPU load, 0/8+ in isolation — a genuine, load-dependent
  bug this project's own new CI would have hit intermittently). Root cause: the claim is a
  sequence of statements (SELECT ... FOR UPDATE SKIP LOCKED, INSERT, UPDATE `next_run_at`), and
  under READ COMMITTED, SKIP LOCKED only prevents two transactions from locking the *same instant*
  — it doesn't serialize the whole multi-statement claim-and-advance sequence against a second
  transaction racing the first. Fixed with a transaction-scoped Postgres advisory lock
  (`pg_advisory_xact_lock`) around the whole sequence — trades per-row concurrent claiming for
  full serialization of tick calls, an acceptable cost since this is a periodic scheduler
  operation, not a high-concurrency hot path. Independently re-verified with 8/8 clean reruns
  under the same load conditions that previously reproduced the bug.
  **Update (still during Phase 11, discovered while verifying item 11.9): the fix reduced but did
  not fully eliminate the race.** Re-running the real `cargo test --workspace` command (not the
  narrower single-background-build repro used to verify the original fix) surfaced the identical
  failure signature once in 4 repeated runs (1/4, down from a deterministic 6/6 pre-fix). The
  `pg_advisory_xact_lock` is confirmed still present and correctly structured in
  `crates/djangors-tasks/src/lib.rs`'s `tick_recurring_tasks` — the remaining gap has not been
  root-caused. Plausible direction for follow-up: `cargo test --workspace`'s default cross-crate
  concurrency (many crates' test binaries as separate OS processes, not just one extra background
  build) may expose a timing window the original repro didn't reach. **Genuinely open, not
  silently dropped** — needs either deeper diagnosis (e.g. live `pg_locks` inspection during a
  captured failure) or an explicit decision to accept this as a rare, documented limitation.
  **Root-caused (2026-07-27, task #60): the `pg_advisory_xact_lock` fix is correct and complete —
  `tick_recurring_tasks` itself is not the bug.** Directly re-verified with ~520 fresh repro
  attempts across three independent methodologies: (1) 100 runs of the existing
  `test_tick_recurring_tasks_dual_claim_race` under real 8-core CPU saturation via `tokio::spawn`
  scheduling jitter, (2) a temporary 400-iteration `tokio::sync::Barrier`-synchronized loop forcing
  the two concurrent calls to start at the same instant (removing scheduling luck as a variable
  entirely) under the same CPU load, (3) 20 more runs of the original test while five other crates'
  full DB-backed test suites (`djangors-orm`/`djangors-migrations`/`djangors-views`/
  `djangors-admin`/`djangors-auth`) ran concurrently against the same `djangors_test` database to
  reproduce genuine cross-binary connection contention. **Zero double-claims and zero lost updates
  across all ~520 runs.** The real mechanism: during that cross-binary run, `djangors-admin`'s and
  `djangors-auth`'s own test suites failed instead — `djangors-admin` hit
  `relation "auth_user" already exists` (42P07) and `djangors-auth` hit `relation "auth_user" does
  not exist` (42P01) plus a cascading `PoisonError` once one test panicked mid-suite. Both crates'
  tests do their own `DROP TABLE IF EXISTS auth_user` / `CREATE TABLE auth_user` dance against the
  *same shared* `djangors_test` Postgres database using the *same global table name*, each only
  serialized by its own crate-local mutex (`djangors-admin`'s `DB_MUTEX`, confirmed present and
  correctly held by every one of its 33 `auth_user`-touching test functions — not a bug there
  either). Neither mutex has any effect across the process boundary between the two crates' separate
  test binaries, which `cargo test --workspace` runs as concurrent OS processes. **Conclusion: the
  previously observed "1/4 under `cargo test --workspace`" was very likely this cross-crate
  shared-table DDL race being misattributed to `tick_recurring_tasks`, not a real fault in it** —
  full-workspace runs surface *some* sporadic failure under load, and the tasks crate's own test
  suite is where it happened to be first noticed and investigated. Closing this item as resolved;
  the newly-confirmed cross-crate table-collision issue is tracked separately (see below) since it's
  a distinct, real bug affecting any crate pair that shares a global table name against the same
  test database, not something specific to background tasks.
  **Correction (2026-07-28): this conclusion was wrong — `test_tick_recurring_tasks_dual_claim_race`
  really does have a genuine, deterministic root cause, unrelated to cross-crate load.** Found while
  investigating an unrelated crates.io republish: running `cargo test -p djangors-tasks --lib`
  *alone*, with zero cross-crate interference, still failed 3 of 5 consecutive runs with the
  identical signature (`enqueued1 + enqueued2 == 2`). The `~1/4` recurrence rate that task #60's 520
  runs happened not to catch is not CPU-load-dependent at all — it's **wall-clock-alignment**-
  dependent: the test set the row's `next_run_at` to an arbitrary non-boundary value (`now - 1
  minute`), which is not a state `next_run_at` can ever actually be in during real operation (both
  `register_recurring` and `tick_recurring_tasks` itself always derive it from
  `Schedule::after(...)`, so it's always boundary-aligned). Whenever the test happened to run within
  roughly the first 60 seconds after a real 5-minute cron boundary, the single-step `next` computed
  from that artificial `previous` would itself still be `<= now`, making a second, concurrent tick
  see the row as legitimately due again — a real occurrence, correctly shared between two workers per
  the SKIP LOCKED catch-up-sharing design, not a duplicate of the first. Task #60's 520 verification
  runs simply never happened to execute inside that ~20%-of-every-5-minutes window. Fixed two things
  in `tick_recurring_tasks` (`crates/djangors-tasks/src/lib.rs`): captured `now` *after* acquiring the
  advisory lock instead of before (a tick that waited on the lock must not compare against a stale
  pre-wait timestamp), and rewrote the test's setup to seed a realistic, boundary-aligned,
  mildly-overdue `next_run_at` (the true most-recent past cron boundary) instead of an arbitrary
  offset — matching the only state the row can actually be in during real operation. Deliberately did
  **not** change the single-step advance-per-tick logic itself: that's the intentional design
  enabling concurrent/successive ticks to divide up catch-up work one occurrence at a time (verified
  this is intentional via the neighboring `test_recurring_task_advances_from_previous_scheduled_time_
  not_now` test, which explicitly asserts a heavily-overdue row stays overdue after one tick). Reran
  the corrected test 20 consecutive times plus the full `djangors-tasks` suite and the full workspace
  suite — all clean, and the fix is no longer wall-clock-luck-dependent (verified by re-deriving the
  boundary math directly, not just by rerunning until it happened to pass).
- [ ] **Cross-crate test DDL races on shared global table names (found via task #60 investigation,
  2026-07-27)** — different crates' test binaries (e.g. `djangors-admin`, `djangors-auth`) each
  `DROP TABLE IF EXISTS` / `CREATE TABLE` the same globally-named table (`auth_user`) against the
  same shared `djangors_test` database, each serialized only by an in-process, crate-local mutex
  that has no effect across the OS-process boundary `cargo test --workspace` runs crate binaries
  across. Confirmed reproducible: running `djangors-admin` and `djangors-auth`'s test suites
  concurrently reliably produced a `relation "auth_user" already exists` (42P07) failure in one and
  a `relation "auth_user" does not exist` (42P01) + cascading `PoisonError` failure in the other.
  Not limited to `auth_user` — any table name shared by two or more crates' test setup code is
  exposed to the same class of race. **Fix direction**: adopt `djangors-test`'s existing
  `TestDatabase::isolated()`/`isolated_url()` per-test-database feature (built in 11.4) more broadly
  in place of the shared `TestDatabase::connect()` most crates currently use for these tests, or
  serialize DB-touching crates' test binaries in CI (e.g. `cargo nextest` with limited
  cross-binary parallelism). Not attempted here — real scope, deliberately left as its own item
  rather than folded into the task #60 investigation that found it.
- [x] **Migration rollback + typed `Operation` variants** — **done (11.1, commit `c6e7e0e`).**
  Fixed the real, confirmed bug where `dj migrate` only ever checked a single hardcoded
  `'0001_initial'` flag and never read/applied any `migrations/NNNN_*.sql` file from disk (every
  `ALTER TABLE ADD COLUMN` migration `makemigrations` ever generated was dead code, never
  executed). Added real per-file migration history tracking, new typed `Operation` variants
  (`AddColumn`/`DropColumn`/`AlterColumnType`/`RenameColumn`/`DropTable`) with `reverse()`/
  `to_down_sql()`, and `dj migrate --rollback [N]`. The codex dispatch's core engine was solid but
  skipped all required tests and its "real Postgres tests passing" self-report didn't correspond
  to any actual test code — wrote the 4 required DB-backed tests myself, which surfaced 2 more
  real bugs: `dj migrate` called `introspect_models()` unconditionally even on the new file-based
  path that doesn't need it, and `rollback_from_dir` queried "most recently applied" globally
  across the whole shared tracking table instead of scoping to the target migrations directory
  (caught via a real, reproducible test failure). Verified end-to-end via the real `dj` binary
  against a scratch project, not just library tests.
- [x] **Model-level signals** (`post_save`/`pre_save`/`post_delete`/`pre_delete`) — **done (11.2,
  commit `c94832e`).** New `ModelSignal<T>` in `djangors-orm` (a duplicate, not a re-export, of
  `djangors-core`'s `Signal<T>` pattern - the dependency direction is `djangors-core → djangors-orm`,
  so `djangors-orm` cannot import from `djangors-core`). Every `#[derive(Model)]` struct gets
  `pre_save_signal()`/`post_save_signal()`/`pre_delete_signal()`/`post_delete_signal()`, wired into
  the generated `save()`/`update()`/`delete()` methods. Took two dispatch attempts: the first
  (codex) correctly refused to fabricate completion when the doc's original `Arc<Self>` payload
  turned out to still require `Self: Clone` to construct; revised the payload to
  `Vec<(&'static str, Value)>` built from the already-existing `Model::field_values()` instead,
  adding zero new trait bounds to any model. Second attempt (opencode, deepseek-v4-flash-free)
  implemented it correctly, including catching and fixing its own test bug along the way.
- [x] **`bulk_create`** — **done (commit `23c4aa6`).** `QuerySet::bulk_create(db, items)` issues a
  single multi-row `INSERT ... VALUES (...), (...), ... RETURNING pk`, skipping auto fields the
  same way `insert_raw` already does. Written directly (small, mechanical).
- [x] **ModelForm-equivalent** — **done (11.5, commit `fdc9854`).** Generated by extending
  `#[derive(Model)]` itself, not a second cross-struct derive (a direct Django transplant hits a
  hard proc-macro limitation — a macro on struct B can't introspect struct A's fields). Each
  model gets a `{StructName}FormCleaned` type plus `validate_form()`/`from_cleaned_form()`/
  `apply_cleaned_form()`, reusing `djangors-forms`' existing field types. Auto/primary-key/
  `FileField` fields are excluded. Found and fixed a real bug: generated `BooleanField`s used
  `required: !nullable`, but `BooleanField::clean()` intentionally treats a required field's
  `false` as "missing" (mirroring Django's checkbox semantics) — a plain non-nullable `bool`
  column can legitimately be `false`, so boolean form fields are always `required: false`
  regardless of model nullability. HTML widget rendering is deliberately out of scope (tracked
  under the generic-CBV item below).
- [x] **Real multipart file upload parsing** — **done (11.6, commit `dd0b672`).** A new
  `Multipart` extractor (`djangors-core`, via `multer`) parses `multipart/form-data` bodies with
  real size limits, splitting fields into `files`/`texts` (Django's `request.FILES`/`request.POST`
  split). `save_upload()` (`djangors-staticfiles`, since it already depends on `djangors-core` —
  the reverse dependency direction would have been a cycle) writes a parsed file through any
  `Storage` backend under a content-hash-based, collision-avoiding path. Wiring this into
  ModelForm/CBVs end-to-end is separate, future work — this ships the parsing + storage-writing
  primitives.
- [x] **Server-rendered generic CBVs** — **done (11.7, commit `f348476`).** New `djangors-views`
  crate: `ListView`/`DetailView`/`CreateView`/`UpdateView`/`DeleteView`, generic over `M: Model +
  FromRow` (+ `ModelForm` for the write views), mirroring `djangors-rest`'s `ViewSet` method shape.
  Required a prerequisite fix to already-shipped code: Phase 11 item 5's ModelForm methods were
  inherent per-struct methods, not a trait, so generic code couldn't call them through a type
  parameter — added a `ModelForm` trait (super-trait of `Model`) that `#[derive(Model)]` now also
  implements by delegating to the existing inherent methods (purely additive, the 5 existing
  ModelForm tests are untouched). Found and fixed 2 real bugs in the dispatch's own test suite: a
  shared-table test race (same fixed-table-name pattern seen in `djangors-tasks`, fixed the same
  way) and a minijinja template using `for k, v in errors` on a JSON object without the required
  `.items()` call.
- [x] **Custom management commands plugin mechanism** — **done (11.8, commit `49e17c7`).** New
  `#[management_command]` attribute macro (mirroring `#[task]`'s wrapper + `inventory::submit!`
  structure) lets a project register `dj <name>`. Since `dj` is a separately-compiled binary that
  can't see a registry populated inside a *different* compiled project, this mirrors the project's
  own existing solution to the identical problem (model introspection): `dj` shells out via
  `cargo run --quiet` with a `DJANGORS_RUN_COMMAND` env var when it sees an unrecognized,
  non-flag subcommand, and the user project's own `main()` (already calling
  `introspect_models_if_requested()`) now also calls `run_management_command_if_requested()`.
  Found and fixed 2 real bugs in the dispatch's implementation: `dj --help`/`--version` were
  being incorrectly intercepted as unknown commands (fixed by excluding flags from the check),
  and the handler spun a second nested tokio runtime inside the already-running one, panicking on
  every real invocation (fixed by making the function async instead). Wrote the required real
  end-to-end test myself: a genuine registered command in `examples/polls`, exercised via the
  actual `dj` binary as a subprocess, confirming an observable side effect.
- [x] **Contenttypes / `GenericForeignKey` framework** — **done (11.9, commit `3338ed8`).** New
  `djangors-contrib-contenttypes` crate: a real `ContentType` model, `sync_content_types()`
  (mirroring `dj createpermissions`'s upsert-per-registered-model pattern, now wired into that
  same command), and `generic_key_for`/`resolve_content_type` forward/reverse lookups. No attempt
  at Rust-side dynamic type-erased fetching (no equivalent of Python's `ContentType.model_class()`)
  — callers capture display info at creation time, the same precedent `djangors-admin`'s `LogEntry`
  already sets. Deliberately does not migrate `djangors-contrib-guardian`/`djangors-admin`'s
  existing bespoke `(app_label, model_name, object_id)` fields to the new abstraction — ships the
  reusable primitive, adoption elsewhere is separate work. **This completes every Phase 11
  Django-parity framework feature.**
- [x] **`TestDatabase` per-test isolation + fixtures loader** — **done (11.4, commit `1d67a54`).**
  Rather than the invasive "wrap each test in one open transaction" rewrite (which would require
  threading a transaction handle through 60+ call sites across 17 crates, since every `QuerySet`
  method takes `&Database` and executes directly through its pool), `TestDatabase::isolated()`
  gives each test a uniquely-named, genuinely separate throwaway Postgres database instead —
  same practical guarantee (no cross-test contamination, ever), zero changes to
  `Database`/`QuerySet`'s existing execution model. `cleanup()` is the primary teardown path
  (terminates lingering connections, `DROP DATABASE`); a `Drop` impl provides a best-effort
  fallback since Rust has no async `Drop`. `load_fixtures<T>()` deserializes a JSON array and
  reuses `QuerySet::bulk_create`. The existing `TestDatabase::connect()` behavior (shared
  persistent database) is untouched — this is additive, not breaking.
- [x] **`CHANGELOG.md`** — **done (commit `ea252fe`).** Written directly from real git history,
  grouped by phase.
- [x] **crates.io publish prep + actual publish** — **prep done (commit `59b07ee`):** added the
  real `LICENSE-MIT`/`LICENSE-APACHE` texts (the workspace claimed `MIT OR Apache-2.0` since day
  one but neither license file ever existed), bumped `0.0.1` → `0.1.0` workspace-wide, stripped
  the `"(placeholder release)"` suffix from 6 crate descriptions. **Publish itself later ran
  successfully** for essentially every crate that existed at the time, via a long-running
  background resilient-publish script — confirmed live on crates.io, not just attempted.
  **Now stale**: that `0.1.0` snapshot predates every Phase 12 fix/feature below, and
  `djangors-pdf` (created mid-Phase-12) was never published at all. Workspace version bumped
  `0.1.0` → `0.2.0` (task #72) to prepare a republish covering every crate, including the four
  never-published ones from this phase (`djangors-pdf`/`djangors-deploy`/
  `djangors-contrib-payments`/`djangors-contrib-tenancy`). **`cargo publish` for `0.2.0` run for
  real and independently verified — done.** Found and fixed a real bug before publishing could
  succeed: 13 crates had internal *dev*-dependencies (e.g. `djangors-auth`'s dev-dependency on
  `djangors-test`) pinned with both `path` AND `version`, left over from the earlier blanket
  version-bump script not distinguishing `[dependencies]` from `[dev-dependencies]`. This makes
  `cargo publish` require the dev-dependency to already resolve against crates.io, which is
  circular when publishing in dependency order (djangors-core and djangors-macros have a mutual
  dev-dependency on each other, for instance) — fixed by stripping `version` from internal
  dev-dependencies, keeping only `path` (a dev-dependency declared this way is dropped from the
  published manifest entirely, which is correct: it's irrelevant to a downstream consumer, only
  used for this workspace's own `cargo test`). Published in real dependency-topological order (32
  crates), waiting for each crate's crates.io index propagation before publishing the next
  dependent one. **Independently verified all 32 crates are genuinely live at `0.2.0`** via direct
  crates.io API requests per crate (not trusted from the publish script's own log) — 0 missing.
  **Found a real bug in this shipped `0.2.0`**: answering the user's own question — how does a
  user use djangors from scratch if they have Rust installed, instead of cloning the repo — proved
  `dj new` was broken for real crates.io installs. It located the sibling `djangors-core` crate via
  `env!("CARGO_MANIFEST_DIR")`, a compile-time constant that only resolves correctly when
  `djangors-cli` is built inside the cloned monorepo; a real `cargo install djangors-cli` build
  resolves it to the cargo registry cache instead, where the directory is suffixed with a version
  (`djangors-core-0.2.0`, not `djangors-core`), so the generated path dependency could never
  resolve and `cargo build` failed immediately on every new project. Fixed with a runtime
  directory-existence check (path dependency if a sibling checkout exists, otherwise a real version
  dependency on the published crate). Since crates.io versions are immutable, shipping the fix
  required a new version: workspace bumped `0.2.0` → `0.2.1`, all 32 crates republished and
  independently verified live again — 0 missing. Verified the fix against the real live registry,
  not just a local simulation: fresh `cargo install djangors-cli --version 0.2.1`, `dj new`,
  `cargo build`, and real HTTP requests to the generated project's `/`/`/healthz` — all green.
- [ ] **Deploy three example apps live** (1.0 Definition of Done). **One of the two is genuinely
  live on Render** — `render.yaml` defines both services with real managed Postgres, healthchecks,
  generated `DJANGORS_SECRET_KEY`, `DEBUG=false`, and the two non-obvious env vars a container
  deploy needs (`DJANGORS_ALLOWED_HOSTS` for Render's public hostname, `DJANGORS_HOST=0.0.0.0`
  because the default loopback bind makes Render's port scanner time out). Verified live
  2026-07-31: `djangors-polls.onrender.com` returns 200 on `/healthz` and `/`, and `/admin/`
  serves a real Djangors auth gate. **`djangors-school.onrender.com` is not up** — it returns a
  bare `Not Found` in ~0.25s on `/`, `/healthz`, and `/admin/`, which is Render's own router
  answering, not the app; the service needs redeploying or was never created from `render.yaml`.
  School is the more valuable of the two to have live, since it is the one with 5,000 seeded rows
  and a realistic admin. Third app not started.
- [ ] Third-party **security audit** — carried over from Phase 10, still genuinely blocked on a
  real budget/vendor decision only the user can make; the internal review already done stands as
  real interim groundwork.

### Phase 12 — Post-1.0 hardening (2026-07-27, ongoing)

Compiled after the first real Render deployment surfaced several genuine bugs, and after
comparing djangors directly against a real production Django SaaS backend
(`school-management-saas-`) to find concrete, non-hypothetical gaps rather than guessing.
Sequenced easiest → hardest; tracked as tasks #62–71.

- [x] **Quote every SQL identifier in `QuerySet`** — **done.** The identifier-quoting bug fixed in
  `derive(Model)`'s INSERT/UPDATE SQL (7.4) and flagged as a known, deliberately-deferred gap in
  `queryset.rs`'s `field_to_col` was still open. Fixed every raw identifier interpolation (5
  `field_to_col` closures, `order_by()`, and every `table_name`/`pk_column`/relation-column
  reference in `insert_raw`/`bulk_create`/`delete_by_pk`/`prefetch_related`). Real regression test
  (`test_filter_order_and_aggregate_on_reserved_keyword_field_names`) exercises `.filter()`/
  `.order_by()`/`.count()` on a model with fields literally named `user`/`group`. Full
  `djangors-orm`/`djangors-rest`/`djangors-admin`/`djangors-tasks`/`djangors-views` suites
  re-verified clean.
- [x] **`#[derive(Settings)]`** — **done.** The Djangors equivalent of `pydantic-settings`/
  `django-environ`: a new proc-macro (`djangors-macros::settings`) generating a `load()` method
  that reads each field from an environment variable (`{PREFIX}_{FIELD}`), coerces it into the
  field's declared type via a new `djangors_core::settings::FromSettingsValue` trait (impl'd for
  `String`, `bool`, every built-in int/float type, `Vec<String>` as comma-separated), supports
  `#[djangors(default = <expr>)]` fallbacks and `Option<T>` fields (`None` if unset), and returns a
  new `SettingsError` (`MissingRequired`/`InvalidValue`, distinguishable) on the first required-but-
  unset or unparseable field. Unlike `DjangorsSettings` (the framework's own fixed settings
  struct), this is for a *user's own app* config — replaces the ad-hoc `std::env::var()` +
  `eprintln!`/`exit(1)` pattern `examples/polls/src/main.rs` used for `DATABASE_URL`. Required
  adding `extern crate self as djangors_core;` (matching the established pattern in
  `djangors-orm`/`djangors-tasks`) since the derive macro's generated code references
  `djangors_core::settings::...` paths, which don't resolve from within `djangors-core`'s own test
  suite without it. 4 real runtime tests (missing-required error, defaults applied, every
  supported type parses from a real env var, invalid-value vs missing-required distinguished).
- [x] **CSP builder middleware** — **done.** `django-csp` equivalent: a `CspBuilder` in
  `djangors-core::middleware` matching `HstsLayer`'s existing style (builder methods per directive
  — `default_src`/`script_src`/`style_src`/`img_src`/`connect_src`/`font_src`/`frame_ancestors`/
  `form_action`/`object_src`, plus a bare `upgrade_insecure_requests()` flag and `.report_only(bool)`
  to send `Content-Security-Policy-Report-Only` instead of enforcing), `.build()` into a `CspLayer`
  tower `Layer`. Directive values pass through verbatim (no source-keyword validation), matching
  `HstsLayer`'s deliberately minimal scope. 4 real tests (directive assembly order, the bare
  `upgrade-insecure-requests` flag, report-only header-name switching, and a real tower-service
  request round-trip asserting the actual response header).
- [x] **Sentry/observability integration** — **done.** `sentry-sdk` equivalent: a new opt-in
  `sentry` Cargo feature on `djangors-core` (`sentry`/`sentry-tracing` crates, matching the
  existing `redis`-on-`djangors-cache` optional-feature convention — zero cost/deps for anyone who
  doesn't enable it). `init_production_logging_with_sentry(dsn)` builds Sentry's client together
  with a layered `tracing_subscriber` (JSON formatting + `sentry_tracing::layer()`) in one call,
  since `tracing` only allows a single global subscriber — bolting Sentry onto an
  already-initialized `init_production_logging()` subscriber isn't possible after the fact.
  `ERROR`-level spans become Sentry events automatically; everything else becomes breadcrumbs.
  Panics are captured via Sentry's built-in panic integration. An empty/invalid DSN produces a
  disabled client (the SDK's own cross-language convention) rather than erroring, so it's always
  safe to call unconditionally from a settings value that may be empty in development. Real test
  confirms the empty-DSN case is genuinely disabled (`guard.is_enabled() == false`) without
  reaching the network. Verified both with and without the feature flag; default (no-`sentry`)
  build unaffected.
- [x] **django-axes-style persistent account lockout** — **done.** `django-axes` equivalent, genuinely
  distinct from the existing `RateLimitedBackend` (in-memory, per-process, throttles the *rate* of
  attempts) — a new `PersistentLockoutBackend<B: AuthBackend>` rejects even *correct* credentials
  once `max_attempts` consecutive failures trip a lockout, persisted in a new `LoginLockout`
  model/table (`auth_login_lockout`, a real `#[derive(Model)]` struct, so `dj makemigrations`
  picks it up automatically — confirmed via a real introspection run against `examples/polls`).
  Survives process restarts and is correctly shared across multiple app instances pointed at the
  same database, closing the gap `RateLimitedBackend`'s own doc comment explicitly called out as
  future work. A successful login clears the failure streak entirely; an expired lockout is
  treated as a fresh streak (starts counting from 1) rather than continuing to accumulate. New
  `AuthError::AccountLocked { retry_after_secs }` variant, deliberately distinct from
  `RateLimited` so callers can tell "throttled" from "locked out" apart. One real end-to-end test
  (real `ModelBackend` + a real seeded user, not a mock): fails 3 times, confirms the *correct*
  password is then rejected with `AccountLocked`, expires the lockout via direct SQL (avoiding a
  real 1-hour sleep), then confirms a subsequent correct login both succeeds and fully clears the
  lockout row. Found and fixed a real bug in the first draft along the way: the "treat an expired
  lockout as a fresh streak" logic was also accidentally suppressing cleanup on the *success* path,
  since it filtered the row to `None` before the success/failure match ran — caught by the test
  actually asserting the row was gone, not just that login succeeded.
- [x] **PDF generation helper** — **done, but scoped differently than `weasyprint`.** New
  `djangors-pdf` crate. Considered a real headless-browser-fidelity HTML+CSS renderer (matching
  `weasyprint` more literally) but confirmed this sandbox has no viable Chrome/Chromium install
  path (only a snap-package transitional wrapper, `chromium-browser`, which needs `snapd` — not
  reliably available in a container/deployment context either), and any deployment would gain an
  implicit browser-engine runtime dependency either way. Built a typed Rust builder API instead —
  `PdfDocument::new(title)` then `.heading()`/`.text()`/`.spacer()`/`.table(headers, rows)`, flowing
  top to bottom across A4 pages with automatic page breaks, `.render() -> Vec<u8>` — matching this
  framework's own "typed Rust API over stringly-typed magic" philosophy (the same reasoning behind
  `ModelForm`) rather than an HTML-string-based approach, and with zero external runtime
  dependency (`printpdf`, pure Rust). Directly serves the actual stated need (report cards,
  invoices, receipts: structured text and tables) without the Chrome dependency risk. 3 unit tests
  plus a working doctest; **independently validated with `poppler-utils`** (`pdfinfo`/`pdftotext` —
  a completely separate PDF implementation from `printpdf`) confirming a real, valid A4 PDF whose
  extracted text matches exactly what was written, in order — not just self-consistency against
  the crate's own output.
- [x] **Malware/AV scan hook for uploads** — **done.** `clamd` equivalent — a new
  `djangors_staticfiles::clamav` module (opt-in `clamav` Cargo feature) implementing `clamd`'s real
  `INSTREAM` wire protocol directly (a `zINSTREAM\0` handshake, then length-prefixed chunks
  terminated by a zero-length chunk — simple enough not to need an external crate). Deliberately
  scans **in-memory bytes** (`ClamAvScanner::scan(&[u8])`), not a file path — `clamd` runs as its
  own OS user and generally can't read arbitrary application-owned paths (confirmed directly:
  `clamdscan` against a path-based scan failed with "Permission denied" in this sandbox), and this
  also means a scan can happen *before* anything is ever written to disk or a `Storage` backend at
  all. **Installed a real `clamav-daemon` in this sandbox and tested against it live** (not
  mocked) — confirmed the standard EICAR antivirus test string is correctly flagged
  (`Eicar-Test-Signature`) and an ordinary file passes clean, both via a raw Python socket script
  first (to nail down the exact wire protocol) and then via the real Rust client; a third test
  forces 4-byte chunking specifically to prove the multi-frame `INSTREAM` implementation
  reassembles correctly, not just a lucky single-write case. 6 tests total (3 pure parsing-logic
  unit tests + 3 against the real daemon, which skip rather than fail if no `clamd` socket is
  present — most dev machines and CI won't have one running). Verified both with and without the
  feature flag; default build unaffected.
- [x] **Fix cross-crate test DDL races properly** (task #61) — **done for the concretely-demonstrated
  case** (`djangors-admin` vs `djangors-auth` on `auth_user`); the general pattern is now a real,
  reusable, verified primitive any other crate can adopt for its own colliding table names.
  Considered `TestDatabase::isolated()`'s per-test-database approach first, but it creates a
  genuinely separate physical Postgres database per test (real overhead across 40+ tests) and each
  test would need cleanup-on-every-exit-path handling (`isolated()`'s guard can't clean up
  automatically the way an in-process guard can, since Rust can't run async code in `Drop`) —
  disproportionate cost for what turned out to need a much lighter fix. Instead added
  `djangors_test::acquire_cross_process_lock(db, name)`: a real Postgres session-level advisory lock
  (`pg_advisory_lock`/`pg_advisory_unlock`), which — unlike an in-process
  `std::sync::Mutex`/`tokio::sync::Mutex` — genuinely coordinates across separate OS processes
  connected to the same Postgres server. Deliberately holds one dedicated `PoolConnection` for the
  lock's lifetime rather than using the pool directly (a session-level lock is tied to whichever
  specific connection acquired it, and a pool hands out an arbitrary free connection per query, so
  acquiring/releasing through the pool wouldn't reliably lock/unlock the same session). Verified the
  primitive itself first, in isolation: a real timing-based test using two independent `Database`
  connections and a `tokio::sync::Barrier` forcing simultaneous acquisition, asserting the full
  acquire→release sequence for each never interleaves — 10/10 clean runs. Applied to
  `djangors-admin` (30 call sites) and `djangors-auth` (7 call sites) via a
  once-per-test-binary-lifetime helper (`tokio::sync::OnceCell` + intentionally leaking both the
  lock and its connection, since the lock needs to be held for the entire test binary's run, not
  per-test — `DB_MUTEX` itself already releases and reacquires between tests, which would leave gaps
  otherwise). **Re-reproduced the exact original collision scenario to confirm the fix**: ran the
  real compiled `djangors-admin` and `djangors-auth` test binaries as genuinely concurrent OS
  processes, 3 rounds — 0/3 failures (previously reliably reproducible), with the run timing itself
  as direct evidence of real serialization (whichever suite has to wait takes ~39-40s instead of
  ~17-23s, every round). **Remaining, not attempted here**: other crates/table-name pairs that
  might share the same class of risk — the primitive is now available for any of them.
- [x] **`dj deploy`** — **first slice done, deliberately left to grow rather than gold-plated.** New
  `djangors-deploy` crate: a `DeployProvider` async trait (`provision`/`deploy`/`status`/`logs`/
  `destroy`) adapted from a separate project's `crush-deploy` architecture but redesigned around
  `DeploySpec`/`DeploymentInfo`/`DeployStatus`/`DeployError`, since `crush-deploy`'s exact signature
  assumes an image-tar-upload model that doesn't fit a GitOps PaaS like Render. Two providers ship:
  `RenderProvider` (drives Render's REST API directly — `/v1/postgres`, `/v1/services` with a Docker
  `serviceDetails`, deploy-trigger + poll loop, `/v1/logs`, destroy — every request shape validated
  against the real API earlier this session deploying `djangors-polls` live) and `SshProvider` (shells
  out to the system `ssh` binary via `tokio::process::Command` rather than adding a native
  `ssh2`/libssh2 dependency — the exact `pkg-config`/native-build pain already hit deploying to
  Render — clones/hard-resets the target repo on the remote host and runs the same
  Dockerfile-based `docker build`/`docker run` flow Render uses). Every value interpolated into a
  remote shell command goes through a `shell_quote()` POSIX-escaping helper plus a `validate_slug()`
  guard on `project_name` as defense in depth; a dedicated test
  (`shell_quote_neutralizes_every_dangerous_character`) proves `$(...)`, backticks, semicolons, and
  embedded quotes are all neutralized. **Verified:** `cargo build`/`test`/`clippy -p djangors-deploy`
  all clean; a real SSH round-trip against this sandbox's own Hetzner VPS (`ssh -i <throwaway
  keypair> localhost "whoami && docker --version"`) confirmed the `ssh`-shell-out approach actually
  works end-to-end for command execution. **Not done, and deliberately deferred:** a full live
  provision→deploy→status→logs→destroy run of `SshProvider` was blocked by this VPS's system
  Postgres only accepting connections from `127.0.0.1`/`::1` (a shared service other projects here
  depend on — not modified), so a deployed container reached via the Docker bridge gateway can't
  reach it without either a throwaway database container or a host-network change; no automated test
  exists yet for `RenderProvider` (API-shape-correct by inspection and prior live use, not covered by
  an automated test); no `dj deploy` CLI subcommand wired into `djangors-cli` yet; Railway/GCP/AWS
  providers not started. Test SSH keypair and `~/.ssh/authorized_keys` modification from this
  exploration were cleaned up/restored before moving on. Picking back up here means: a disposable
  Postgres container for the SSH smoke test, a `RenderProvider` test (mockable via a local HTTP
  server), and the CLI wiring.
- [x] **`djangors-contrib-payments`** — **Paystack done.** A provider-agnostic `PaymentProvider`
  async trait (`initiate`/`verify`/`verify_webhook_signature`/`refund`) plus `PaystackProvider`, and
  an idempotency-key-first `Transaction` model (`reference` has a real DB-level UNIQUE constraint,
  not an application-level check-then-insert, which would race under concurrent webhook
  redeliveries or double-clicks). Amounts are `i64` minor units (kobo/cents) everywhere, never a
  float and never `rust_decimal::Decimal` — this satisfies Part 6 item 1's "money is Decimal, never
  float" principle by a different, equally-correct route: djangors-orm's `#[derive(Model)]`
  currently has a hard compile error for `Decimal`/`NaiveDate`/`NaiveTime`/`Duration`/`Uuid` fields
  needing INSERT/UPDATE codegen (a real, tracked gap — see `crates/djangors-macros/src/model.rs`
  ~L389-402), and integer minor units is the same convention Paystack's and Stripe's own APIs use on
  the wire anyway, so there's no float-precision problem to solve with Decimal here in the first
  place. Real Paystack API shapes (not guessed): validated directly against
  `/root/dev/GoGo/backend/internal/pkg/payment/paystack.go` and its webhook handler — a real,
  deployed, working Nigerian fintech integration on this same machine — for `initialize`/`verify`/
  webhook-signature-verification, plus a web search confirming the `refund` endpoint's real
  `data.transaction.reference` + `data.status` shape (not in the GoGo reference, since that project
  handles refunds as an internal wallet credit rather than a real Paystack refund call). Webhook
  handling replicates the exact real order of operations from GoGo's own production handler: read
  raw body bytes → verify HMAC-SHA512 signature against the *raw* bytes (via `hmac`/`sha2`'s
  constant-time `verify_slice`, never a manual `==`) *before* parsing any JSON → only then parse →
  require both `event == "charge.success"` AND `data.status == "success"` → idempotently
  record/update the transaction. Paystack's `amount` field is a JSON number in some responses and a
  string in others (a real, confirmed API inconsistency, not a hypothetical) — handled with a custom
  serde deserializer tested against both shapes.

  Built via this project's dispatch workflow (design-doc-first, independent review after) — first
  attempt via `agy`/gemini died immediately on a quota wall (confirmed via a real `git status`/`git
  diff` check showing zero changes, not assumed), `codex` was blocked by this session's own
  permission classifier on its required `--dangerously-bypass-approvals-and-sandbox` flag (the user
  chose to skip it rather than force through), so this landed on
  `opencode run -m opencode/deepseek-v4-flash-free` — which itself first stalled on an
  over-broad single-shot prompt (read 5 large reference files, produced zero writes, exited clean —
  a known free-tier failure mode of running out of step budget on reads before ever writing) before
  succeeding once split into two leaner dispatches with every API fact inlined directly in the
  prompt instead of pointing at files to go read.

  **Independent review caught one real bug the dispatch's own self-report missed**: `mark_transaction_status`
  called `Transaction::save()` (djangors-orm's save() is INSERT-only, by convention) on an
  already-persisted row instead of `update()`, which only surfaced as a real duplicate-key-violation
  failure once tests were re-run with `DATABASE_URL` actually set — the dispatch's own test run (and
  my first independent rerun) had silently taken the tests' `TestDatabase::isolated()`-unavailable
  skip path with no live DB configured, passing trivially without exercising the real update path at
  all. Fixed directly, then reverified twice against a real live Postgres connection with all 10
  tests genuinely passing (confirmed via wall-clock timing: ~1.6-1.7s with a real DB vs ~0.03s on
  the skip path — solid evidence of genuine DB I/O, not just a green checkmark). Also found and
  fixed, in the same review pass, a pre-existing (unrelated to this dispatch) clippy
  `redundant_closure` regression in `djangors-migrations/src/operation.rs` blocking the full
  workspace `-D warnings` gate, and recovered from a genuine disk-space exhaustion (`target/` had
  grown to 18GB across this session's many builds, filling the VPS to 0 bytes free and causing a
  real, unrelated Postgres `no space left on device` test failure) via `cargo clean`, freeing 19.4GB.

  **Not done, deliberately deferred**: Stripe/Anchor/Moniepoint providers (Paystack first per the
  user's own sequencing); no HTTP route/view wiring into any example app (this crate exposes
  functions an application's own handler calls, matching how `djangors-auth` exposes backends rather
  than routes — deliberate, not an oversight); no idempotency-key middleware for generic POST APIs
  (Part 6 item 4 — a separate, broader concern than this crate's own reference-based idempotency).
- [x] **Multi-tenancy support** — **first slice done** (design doc:
  `docs/design/12.1-multi-tenancy.md`). Grounded the whole design in the user's own real
  `school-management-saas-` Django backend rather than guessing: confirmed shared-schema,
  row-level tenant-FK scoping is the proven real-world model (not schema-per-tenant or
  database-per-tenant, both of which `djangors-migrations` has no orchestration support for
  anyway), and that its own `TenantScopedModelViewSet` requires every subclass to hand-write its
  own `get_queryset()` filter (`school=user.school`, copy-pasted 6+ times in one file) - exactly
  the "discipline instead of a guarantee" gap this slice closes. New `djangors-contrib-tenancy`
  crate: `Tenant` model, a `TenantMembership` join model (a user can belong to more than one
  tenant - more general than the reference project's direct-FK-on-`User` simplification, which
  only works because Django lets you swap `AUTH_USER_MODEL` entirely; djangors-auth's `User` is a
  fixed struct this design deliberately never touches), `TenantResolutionLayer` (reads
  `X-Tenant-Id`, verifies the requesting user actually has a real `TenantMembership` row for it
  before trusting the header - never a default/fallback tenant), and `tenant_scope()`, a one-line
  helper for `djangors_rest::Scoped` implementations that reuses the framework's existing
  `Scoped`/`ScopedViewSet` enforcement mechanism (a required trait bound, not an opt-in convention)
  rather than building new enforcement machinery from scratch.

  Built via dispatch (`opencode/deepseek-v4-flash-free`, after `agy` hit its quota wall again and
  `codex`'s `--dangerously-bypass-approvals-and-sandbox` got blocked by this session's own
  permission classifier - fixed separately by switching to `-c approval_policy=never -s
  workspace-write`, a safer non-interactive invocation, for future dispatches). **Independent
  review caught a real, serious wiring bug the dispatch's own passing tests couldn't have caught**:
  `tenant_scope()` read `req.state::<CurrentTenant>()` (app-wide state, set once via
  `Router::with_state`), but `TenantResolutionLayer` sets `CurrentTenant` via
  `req.extensions_mut()` (per-request extensions, which is what `Router::dispatch` actually
  propagates into `Request::ext()` - confirmed directly in `djangors-core/src/router.rs`). These
  are two different storage mechanisms entirely; every real request would have silently rejected
  even a fully valid, membership-verified tenant. The dispatch's own crate tests didn't catch it
  because they test `TenantResolutionLayer` and `tenant_scope()` in isolation, never through one
  real `Request` exercising both - the existing `CurrentOwner` example in `djangors-rest`'s own
  tests uses `.state()` only because that specific test bypasses the router entirely and hand-
  constructs its `Request`, which isn't representative of how a real middleware connects to a real
  handler. Fixed, and added a new end-to-end test
  (`tenant_scope_reads_current_tenant_set_via_extensions_like_real_middleware_does`) that
  constructs one `Request`, populates `CurrentTenant` via `Extensions` exactly as
  `TenantResolutionLayer` does, and proves `tenant_scope()` both resolves it AND genuinely returns
  only the current tenant's own row out of two real tenants in the database - the actual
  cross-tenant isolation guarantee this whole crate exists for, which no other test here proved
  directly. Also found and fixed a test-hygiene gap (fixed slugs/no cleanup meant re-running the
  suite against the same persistent dev database hit stale unique-constraint violations from a
  prior run) by truncating the tables on each test's setup. All 7 tests reverified passing twice in
  a row against a live, freshly-recovered Postgres (see below).

  **Unrelated but real environment incident surfaced during this verification pass**: this
  session's own repeated full-workspace builds had filled the VPS's disk to 0 bytes free earlier;
  Postgres's checkpointer hit `PANIC: could not write to file "pg_logical/replorigin_checkpoint.tmp":
  No space left on device` and shut itself down cleanly, and its own crash-recovery attempt then
  *also* hit `could not extend file ...: No space left on device` before disk was freed via
  `cargo clean`, leaving the whole system Postgres cluster down (`pg_lsclusters` showing
  `down`) - a real, shared-service outage this session's own build activity caused, not a
  pre-existing bug. Restarted via `systemctl start postgresql` once disk space was available again;
  WAL redo completed and the cluster came back online cleanly (confirmed via the log: `redo done`,
  `checkpoint complete`, `database system is ready to accept connections`, plus a live `SELECT 1`).

  **Not done, deliberately deferred** (see the design doc's own scope section): schema-per-tenant/
  database-per-tenant support, automatic Postgres Row-Level Security policy generation (documented
  as a manual advanced option only), subdomain-based tenant resolution (v1 is `X-Tenant-Id`-header
  only), a `#[djangors(tenant_scoped)]` derive-macro shortcut (the plain `tenant_scope()` helper
  covers the same need with much less risk), `djangors-admin` tenant-scoping integration (the
  design doc's own highest-value follow-up piece - not started), and cross-tenant reporting
  tooling.

### Phase 13 — SQLite as a real second backend (2026-07-28 → 07-30, shipped in 0.5.0/0.6.0)

Design docs `13.1`–`13.4`. Postgres was the only backend that worked end to end, which made
`dj new` → run a nontrivial lie for anyone who did not already have Postgres installed.

- [x] **13.1 SQLite backend in the ORM + `djangors-db`.** `Dialect` enum with `placeholder`,
  `ilike`, `quote_ident`, `cast_float`, `extract_date_part`, `bytea_type`. Enum dispatch (`Conn`,
  `DbRow`) rather than generics — chosen deliberately, because generic `DB: Database` bounds
  propagate `for<'r> i64: Decode<'r, DB>` obligations into every caller and the blast radius was
  unacceptable.
- [x] **13.2 Dialect-aware raw SQL** across every production call site.
- [x] **13.3 Dialect-aware migrations + `makemigrations`.** Added `auto_pk_type()`,
  `timestamp_type()`, `current_timestamp()`, `from_url()`, and `Database::transaction_conn()`.
  **Scope-estimate lesson worth keeping:** the initial estimate ("two places in `plan.rs`") came
  from grepping only for `Dialect::Postgres` and missed the migration runner entirely; the real
  surface was 5 files plus a blocker — `Database::transaction` returns `Err` on SQLite, so a new
  `transaction_conn` had to be added alongside it rather than changing it. Grep for the *absence*
  of dialect awareness, not for its presence.
  `AlterColumnType` returns `MigrationError::UnsupportedOnDialect` on SQLite (no 12-step rebuild).
- [x] **13.4 Documentation pass.** Comment pass over the ten thinnest crates plus eight new topic
  guides (`databases`, `migrations`, `tasks`, `caching`, `i18n`, `sessions`, `static-files`,
  `signals`). Review used a Rust comment-stripper (respecting string/char/raw-string literals) to
  mechanically prove the edits were comments-only — 0 of 13 files showed a code change. A first
  attempt at this gate with a regex produced false positives on comments containing `"`; the
  mechanical invariant is what made the pass trustworthy, not the reading.

### Phase 14 — Test-suite portability (2026-07-30 → 07-31, shipped in 0.6.0)

- [x] **14.1 Dual-backend test harness.** `TestBackend::{Postgres, Sqlite}`,
  `TestDatabase::new_for_backend()`, in-memory SQLite. Measured: `djangors-admin` 32 tests in
  **0.69s** vs 15.8s on Postgres; `djangors-rest` 45 tests in **0.44s** vs 4.7s.
  **The load-bearing detail:** `sqlite::memory:` gives every pooled connection a *separate*
  database, so the pool is pinned to `max_connections(1)`; without it, setup DDL on one connection
  is invisible to the next query and the failure presents as a baffling "no such table."
  **Process note:** this was open for three phases because 13.1/13.2/13.3 each scoped test SQL out
  and that exclusion was carried forward unexamined, even though "tests would run faster" was the
  original stated reason for wanting SQLite at all. The gate that now catches silent test loss is
  diffing test *names*, not counts — a dispatch had previously deleted three transaction tests to
  get a green run.
- [ ] **Run the SQLite leg in CI.** Not done — see the Phase 2 test-harness item. This is the
  cheapest open item in the plan and the one that converts 0.6.0's headline feature from
  hand-verified to machine-verified.
- [ ] **Un-skip the five silently-passing tests.** Isolation levels, advisory locks, and
  `SKIP LOCKED` early-return when `DATABASE_URL` is unset and **report as passing on SQLite
  without executing.** Either make them `#[ignore]` on SQLite so the count reflects reality, or
  have the harness fail loudly when asked for a Postgres-only test on a SQLite backend. As
  written, they are the one thing in the suite that can report green while covering nothing.

### Release history (crates.io + GitHub Releases)

| Version | Date | Contents |
|---|---|---|
| 0.2.1 | 2026-07-27 | `dj new` path-dependency fix, republished all 32 crates |
| 0.5.0 | 2026-07-30 | Phase 13.1–13.3 — SQLite across ORM, raw SQL, migrations |
| 0.6.0 | 2026-07-31 | Phase 13.4 + 14.1 — docs pass, dual-backend test harness |

**Standing publish routine** (learned the hard way — crates.io publishes before 2026-07-28 created
no git tags or GitHub Releases at all): bump → `verify` no stale pins → publish in dependency
order → **independently confirm via the crates.io API, not the publish exit code** → push → tag →
create the GitHub Release. Two traps that have each bitten once: the crates.io API returns nothing
useful without a `User-Agent` header, and a `djangors[a-z-]*` sed class silently skips
`djangors-i18n` because of the digits, leaving a stale pin that only surfaces as a build failure
much later.

---

## Part 6 — What "banking / schools / e-commerce grade" concretely requires

Bake these into the phases above; this list is the acceptance criteria for the pitch:

1. **Money is `Decimal`, never float**; currency-aware field in contrib.
2. **`select_for_update`, explicit isolation levels, savepoints** — documented double-entry-ledger example in the docs. **Status as of 0.6.0: only explicit isolation levels exist.** There is no QuerySet `select_for_update` and no `SAVEPOINT` support anywhere in the workspace (see Phase 2). Two of the three concurrency primitives this pitch rests on are missing, so the double-entry-ledger example cannot honestly be written yet. This is the largest single gap between the banking pitch and the code.
3. **Audit trail default-on** (contrib-audit): who changed what, when, from which IP/request-id; immutable log table option.
4. **Idempotency-key middleware** for POST APIs (e-commerce checkout, payment webhooks).
5. **Object-level permissions + row scoping** (a teacher sees only their class; a branch officer only their branch) with a documented pattern.
6. **2FA for admin users**, session timeout policies, password policy settings, login throttling.
7. **PII tooling:** field-level encryption-at-rest helper, `#[djangors(sensitive)]` masking in logs/error pages/audit diffs.
8. **Compliance posture docs:** how Djangors's defaults map to OWASP ASVS / SOC2 controls (auditors love a mapping table).
9. **Zero-downtime migrations guide** (additive-first patterns, `--check` in CI, lock-timeout settings for Postgres DDL).
10. **Observability:** `tracing` spans per request/query out of the box, OpenTelemetry exporter feature, slow-query log, `/health` + `/ready` endpoints.

---

## Part 7 — Project operations (do these alongside the code)

- **Cadence:** ship a tagged 0.x release every 4–6 weeks from Phase 1 onward, each with a changelog and one demo GIF. Momentum is marketing.
- **RFC process:** any public-API change ≥ medium gets a short design doc + 48h comment window (even while it's just you — future contributors inherit the archive).
- **Issue hygiene:** `good-first-issue` labels from Phase 2 onward; contrib crates (Phase 7) are the designated community on-ramp.
- **Testing bar:** every crate ≥ 80% coverage; ORM and migrations get property-based tests (proptest: random model-edit sequences must always produce applicable migrations); fuzzing on all parsers.
- **Dogfooding:** the project website's own tiny CMS runs on Djangors as soon as the admin exists.
- **Funding path (later):** GitHub Sponsors → hosted admin/support offering → never gate core features.

## Part 8 — Top risks and their mitigations

| Risk | Mitigation |
|---|---|
| Scope drowning (this is 2+ person-years) | Phases are strictly ordered; the polls example is the scope-cutter — if polls doesn't need it before 1.0, it's post-1.0. |
| Compile times ruin the "pleasant" promise | Design 4.5 is a *tracked benchmark in CI*, not a hope. Runtime templates carry most iteration. |
| Proc-macro complexity explodes (djangors-macros) | Macros stay thin: they only *emit metadata and delegate* to runtime code in djangors-orm; all logic lives in normal, testable functions. Use `trybuild` for macro error-message tests — good compile errors are a feature. |
| ORM correctness bugs destroy trust with exactly your target users | Run the ORM test suite against real databases in CI from day one; port relevant chunks of Django's own ORM test suite (it's a free, battle-hardened spec). |
| "Yet another Rust web framework" dismissal | Never market the router. Market the **admin + migrations + batteries**. Demos, not benchmarks, lead every post. |
| Burnout | The 4–6-week release ritual + visible example apps give constant finish-lines. Recruit 1–2 co-maintainers by Phase 3. |

## Part 9 — Start-today checklist (first 7 days)

1. ✅ Name research done (2026-07-17): `rango` and `rjango` are taken; **`djangors` is free — publish placeholder crates (`djangors`, `djangors-orm`, `djangors-admin`, `djangors-cli`, `djangors-macros`, `djangors-forms`, `djangors-rest`) today.** Do NOT use `django`/`django.rs` without reading the DSF trademark policy first (see naming row).
2. Create GitHub org + repo, push this plan, add LICENSE/CoC/CONTRIBUTING/SECURITY.
3. Scaffold the workspace (Part 3 layout, empty crates) + CI green.
4. Write `examples/polls` as the aspirational API spec (Part 3 code).
5. Write design docs 4.1 (ModelMeta) and 4.2 (QuerySet) — these unblock everything.
6. Read `django/db/migrations/autodetector.py` and cot.rs's model macro; take notes into `docs/prior-art.md`.
7. Begin Phase 1: Request/Response over hyper + the router.
