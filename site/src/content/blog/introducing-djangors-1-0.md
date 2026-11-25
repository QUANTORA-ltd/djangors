# Djangors 1.0: Batteries-Included Web Development in Rust

After dozens of releases, 38 workspace crates, and full end-to-end verification across PostgreSQL 16 and SQLite, **Djangors 1.0.0** is stable, published on [crates.io](https://crates.io/crates/djangors), and backed by Semantic Versioning guarantees.

If you love Django's productivity—the ORM, migrations, auto-generated admin, forms, auth, REST framework, and `manage.py` workflow—but want Rust's type safety, predictable latency, and single-binary deploys, Djangors 1.0 was built for you.

---

## What's Inside Djangors 1.0

### 1. Multi-Backend ORM, Multi-DB Routing & PostGIS (`djangors-orm`, `djangors-contrib-gis`)
- **PostgreSQL, SQLite, and optional MySQL (`mysql` feature)** behind one unified `QuerySet` API.
- **Primary/replica & multi-DB routing** via `DatabaseRouter`, `PrimaryReplicaRouter`, and `QuerySet::using`.
- **Rich lookups & expressions**: `IStartsWith`, `IEndsWith`, `Range`, date/time extractors (`Year`, `Month`, `Day`, `WeekDay`, `Date`, `Hour`), JSON lookups (`JsonHasKey`, `JsonContains`), `Cast`, `Extract`, `Trunc`, `Case`/`When`, `Window` functions, `Subquery`, and `TrigramSimilarity`.
- **Set operations & bulk helpers**: `distinct_on`, `defer`, `only`, `union`/`intersection`/`difference`, `in_bulk`, `bulk_update`, `earliest`/`latest`, `raw`, and `iterator`.
- **Geospatial support**: `djangors-contrib-gis` (`gis` feature) for PostGIS geometry types and spatial queries.

### 2. Schema Migrations & Zero-Downtime Upgrades (`djangors-migrations`)
- Declarative SQL migrations with `dj makemigrations`, `dj migrate`, `dj sqlmigrate`, `dj showmigrations`, and `dj squashmigrations`.
- Supports `--fake`, `--fake-initial`, `--plan`, topological `-- dependencies:`, and typed `RunRust` / `DataMigration` hooks.

### 3. Auto-Generated Admin + AdminDocs (`djangors-admin`)
- Turnkey admin interface with `ModelAdminConfig`, `SimpleListFilter`, changelist facet counts, autocomplete endpoints, custom actions, and `/admin/doc/` AdminDocs introspection.

### 4. Auth, Sessions & Zero-Downtime Django Migration (`djangors-auth`)
- Pluggable `PasswordHasherRegistry` supporting **Argon2id** (`$argon2id$...`) alongside exact Django **`pbkdf2_sha256$...`** and **`pbkdf2_sha512$...`** formats—with transparent re-hashing to Argon2id on user login.
- Optional passkeys (WebAuthn), OAuth 2 / OIDC client & provider, SAML SSO, LDAP, 2FA/TOTP, and object-level permissions (`djangors-contrib-guardian`).

### 5. Forms, FormSets, Templates & i18n/l10n
- 17 standard Django form fields, 8 widgets, `FormMedia`, and `ManagementForm` / `FormSet` / `InlineFormSet`.
- Context processors, `{% querystring %}`, `render_partial` (`{# partialdef #}`), task-local timezone switching, `i18n_patterns`, and `makemessages` / `compilemessages`.

### 6. REST Framework, OpenAPI 3.1 & TypeScript Codegen (`djangors-rest`, `dj`)
- Typed serializers, viewsets, filtering/ordering/pagination, API key & JWT auth, OpenAPI 3.1 spec generation, and `dj openapi --typescript` for instant typed frontend clients.

---

## Get Started in 30 Seconds

```bash
cargo install djangors-cli
dj new mysite && cd mysite
cargo run
```

Explore the [full documentation](/docs/), read the [Django comparison](/compare), or check out our [Rust build & test speed secrets](/blog/rust-secrets-fast-builds-tiny-target-4s-tests/).
