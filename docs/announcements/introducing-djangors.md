# Introducing Djangors: The Django of Rust

*Draft announcement, ready for HN/Reddit/This Week in Rust/a blog post whenever you decide to
publish. Written honestly against the project's actual current state (pre-1.0, under active
development) — nothing here claims 1.0 has shipped, an audit has happened, or anything isn't
true yet. Update the bracketed placeholders before publishing.*

---

If you've built anything in Django, you know the feeling: models describe your data, `manage.py`
handles the busywork, the admin gives you a working back office for free, and the framework has an
opinion — a good one — about sessions, CSRF, migrations, and forms, so you don't have to invent
your own. It's the most productive web framework most of us have ever used.

It's also written in Python, which means the things Python is bad at — a typo'd field name only
caught in production, the GIL capping your concurrency, shipping a whole virtualenv instead of a
binary — are things you just live with.

**Djangors** is an attempt to keep everything that makes Django pleasant and swap the runtime out
from under it. Same shape — models, migrations, admin, forms, auth, a REST framework, background
tasks, i18n, the works — built in Rust, so a typo becomes a compile error instead of a 2am page, a
database `WHERE` clause is generated from a query the type-checker validated, and `cargo build
--release` produces one static binary you copy to a server and run. No GIL, no GC pauses, real
async concurrency underneath.

## What's actually built

This isn't a routing library with an "admin panel coming soon" README. As of today:

- **A real ORM** — `QuerySet`, filter/order/aggregate expressions, `select_related`/
  `prefetch_related` for eager loading, cursor and offset pagination, migrations with real
  autogeneration (`dj makemigrations`/`dj migrate` diff your models against the database).
- **A real admin site** — changelist, filters, search, bulk actions, inline editing, CSV export,
  an audit log, object history — generated from your models the same way Django's is.
- **A real auth system** — users, groups, permissions, session-backed and token/JWT auth,
  password hashing, rate-limited login, CSRF protection with the same header-first/form-fallback
  mechanism Django uses.
- **A real REST framework** — generic serialization, `ViewSet`s, permission classes, OpenAPI 3.1
  generation, and (new) compile-time-enforced scoped viewsets: a model that doesn't implement the
  mandatory-scoping trait simply won't compile against a scoped endpoint, which is a stronger
  guarantee than Django's own runtime `NotImplementedError` pattern it's inspired by.
- **The rest of the batteries**: background tasks (Postgres-backed, `SELECT ... FOR UPDATE SKIP
  LOCKED`, now with cron-style recurring schedules), a template engine with Django-style filters,
  email backends, a cache layer (in-memory/database/Redis), i18n, static files with a pluggable
  storage backend (local disk today, S3 as of this week), named/scoped rate limiting per endpoint,
  and a `dj` CLI that mirrors `manage.py` command-for-command.

## Honest numbers, not marketing numbers

We benchmarked Djangors against a minimal axum app (the "no framework, just routing" baseline) and
against Django/Gunicorn, using [`oha`](https://github.com/hatoo/oha), real Postgres, and a real
5-row indexed query — [full methodology and raw output here](../src/benchmarks.md):

| | Djangors | axum (bare) | Django/Gunicorn |
|---|---|---|---|
| Routing only (`/hello/`) | 60,890 req/s | 78,447 req/s | 831 req/s |
| Real Postgres query | 7,290 req/s | 9,503 req/s | 26 req/s |

Djangors is slower than bare axum — 22% on routing, 23% on the full-stack path — because a
framework that gives you an ORM, middleware, and an admin site isn't free, and we say so plainly in
the docs rather than reframing a miss against our own stated target. It's still 70-280x faster than
the framework it's modeled after.

## What's honestly still missing

This is pre-1.0 and we're not pretending otherwise:

- **No third-party security audit yet.** We've hardened CSRF, sessions, auth rate-limiting, and
  the admin's permission checks, but "we reviewed our own code" isn't the same claim as an
  independent audit, and we won't call this 1.0 until one has actually happened.
- **Not on crates.io yet.** You'd build from source today, not `cargo add djangors`.
- **API freeze is in progress, not complete.** We've published a real
  [stability/deprecation policy](../src/api-stability.md) and frozen the three largest crates
  (`djangors-core`, `djangors-orm`, `djangors-rest`); the remaining ~23 crates are a planned
  follow-up pass, not yet covered by any compatibility guarantee.
- **No production deployments to point to yet** beyond our own example apps.

## Try it

```
git clone https://github.com/QUANTORA-ltd/djangors
cd djangors
cargo build --workspace
cargo run -p djangors-cli -- new mysite
```

[Tutorial](../src/tutorial/01-requests-and-responses.md) ·
[Djangors for Django developers](../src/django-comparison.md) ·
[Topic guides](../src/guides/) ·
[Roadmap](../../PLAN.md)

We'd rather hear "this broke for me" now, before 1.0, than after. Issues, questions, and PRs are
all welcome at [github.com/QUANTORA-ltd/djangors](https://github.com/QUANTORA-ltd/djangors).

---

*[Placeholder: author byline / date / links to HN/Reddit threads once posted]*
