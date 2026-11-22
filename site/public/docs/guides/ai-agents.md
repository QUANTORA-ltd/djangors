# Quick Reference for Developers & AI Agents

This page is a single-page, copy-pasteable cheat sheet for both human developers and AI coding
agents working with Djangors `1.0`.

## 1. Crate & Feature Map

Most applications depend only on the `djangors` facade crate (plus `djangors-macros` for derives):

```toml
[dependencies]
djangors = "1.0"                  # default includes postgres, sqlite, migrations, rest, admin, auth, forms, sessions, template, staticfiles, s3, cache, mail, i18n, tasks, pdf, payments, tenancy, history, ws, password-strength, resilience, secrets
djangors-macros = "1.0"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

Opt-in facade features: `mysql`, `sites`, `gis`, `orgs`, `oauth-client`, `oauth-provider`, `saml`,
`passkeys`, `ldap`, `captcha`, `redis`, `debug-panel`, `otel`, `secrets-remote`.

## 2. Core Patterns

### Define a Model (explicit primary key required)

```rust,illustrative
use djangors_macros::Model;
use djangors_orm::Model;

#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[djangors(app = "blog", table_name = "blog_post", ordering = "-id")]
pub struct Post {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 200)]
    pub title: String,
    pub published: bool,
}
```

### QuerySet (ORM)

```rust,illustrative
use djangors_orm::{q, CompareOp, FuncExpr, Model, SetOp};

// Filter, order, distinct, paginate, and execute
let posts = Post::objects()
    .filter(q!(published = true))?
    .exclude(q!(title = ""))?
    .distinct()
    .defer(&["body"])
    .order_by(&["-id"])
    .limit(20)
    .all(&db)
    .await?;

// Bulk helpers & multi-DB
let by_id = Post::objects().in_bulk(&db, Some(&[1, 2, 3])).await?;
Post::objects().bulk_update(&db, &posts, &["title"]).await?;
let replica_posts = Post::objects().using("replica").all(&db).await?;
let raw_posts = Post::objects().raw(&db, "SELECT * FROM blog_post WHERE published = true", &[]).await?;
```

### Views, FileResponse & Middleware

```rust,illustrative
use djangors_core::{
    ConditionalGetLayer, DjangorsError, FileResponse, LoginRequiredLayer, PathParams, Request,
    Response, Router, StatusCode,
};

async fn health(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "ok"))
}

let router = Router::new()
    .get("/health", health)
    .layer(ConditionalGetLayer::new())
    .layer(LoginRequiredLayer::new("/accounts/login/"));
```

### Typed REST Endpoints + OpenAPI 3.1

```rust,illustrative
use djangors_core::Router;
use djangors_macros::{endpoint, JsonSchema};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, JsonSchema)]
pub struct CreatePost { pub title: String }

#[derive(Serialize, JsonSchema)]
pub struct PostOut { pub id: i64, pub title: String }

#[endpoint(POST, "/posts", request = CreatePost, response = PostOut, status = 201, tag = "posts")]
pub async fn create_post(_req: djangors_core::Request, body: CreatePost) -> Result<PostOut, djangors_core::DjangorsError> {
    Ok(PostOut { id: 1, title: body.title })
}
```

### Auth & Pluggable Password Hashers

```rust,illustrative
use djangors_auth::{
    auth_routes, Argon2Hasher, PasswordHasherRegistry, Pbkdf2Sha256Hasher,
};

// Transparently migrate Django PBKDF2 passwords to Argon2id on login:
let mut registry = PasswordHasherRegistry::new();
registry.register(Argon2Hasher::default());
registry.register(Pbkdf2Sha256Hasher::default());

// Mount turnkey HTML login/logout/password-change/password-reset routes:
let router = Router::new().nest("/accounts", auth_routes());
```

### Templates, Partials & Context Processors

```rust,illustrative
use djangors_template::TemplateEngine;

let mut engine = TemplateEngine::new(vec!["templates".into()])?;
engine.add_context_processor(|req| {
    let mut map = std::collections::BTreeMap::new();
    map.insert("path".to_string(), minijinja::Value::from(req.uri().path()));
    map
});
// In templates: {{ querystring(page=2) }}, {{ val|json_script("data") }}, {# partialdef card #}...{# endpartialdef #}
```

### Testing (SQLite in-memory or Postgres)

```bash
TEST_BACKEND=sqlite cargo test
```

```rust,illustrative
use djangors_test::{assert_num_queries, LiveServer, RequestFactory, SettingsOverride};
```

## 3. Rules & Common Pitfalls

1. **Every `#[derive(Model)]` struct must declare an explicit `#[djangors(primary_key, auto)]` field** (e.g. `pub id: i64`).
2. **Database connections are passed explicitly (`&db`)** — there is no global mutable database singleton.
3. **Use `TEST_BACKEND=sqlite`** for fast local unit/integration tests without a running Postgres server; set `DATABASE_URL=postgres://...` to exercise Postgres-specific features (FTS, `SKIP LOCKED`, PostGIS, advisory locks).
4. **Always run `dj check --deploy`** before production deploys to verify `DEBUG = false`, `SECRET_KEY` length, `ALLOWED_HOSTS`, security headers, and registered system checks.
5. **Never hardcode raw SQL strings with user input** — always use `q!(...)`, `QuerySet`, or `BindValue` parameters.
6. **Keep binaries & `target/` small** — use `djangors = { version = "1.0", default-features = false, features = [...] }` with only the features you need, and keep `[profile.dev.package."*"] debug = 0` + `[profile.release] opt-level = "z", lto = "thin", strip = true, codegen-units = 1, panic = "abort"` (generated automatically by `dj new`).
