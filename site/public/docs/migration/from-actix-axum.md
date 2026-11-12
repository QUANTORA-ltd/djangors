# Moving from Actix Web or Axum

This guide is for Rust developers who already know Actix Web or Axum and want
to build a Djangors application without losing the patterns they already use.
It is also useful for beginners because every section answers the same
question: **where does this code belong?**

Djangors is still Rust, Tokio, Hyper, and Tower underneath. The difference is
that the framework supplies more of the application layer: an ORM, migrations,
forms, authentication, admin, templates, REST ViewSets, tasks, testing tools,
and a project CLI.

If you are coming from Django instead, start with the
[Django comparison guide](../django-comparison.md); this guide focuses on the
Rust web-framework translation.

## The translation in one table

| Concern | Actix Web / Axum | Djangors |
| --- | --- | --- |
| Handler | `async fn` with extractors | `async fn(Request, PathParams) -> Result<Response, DjangorsError>` |
| Router | `App` / `Scope` or `Router` / `nest` | `Router::new()`, `.get()`, `.post()`, `.mount()` |
| Application state | `web::Data<T>` / `State<T>` | `Router::with_state(T)` and `Request::require_state::<T>()` |
| Path values | `Path<T>` | `PathParams::get_as::<T>("name")` |
| Query values | `Query<T>` | `Request::query("name")` or a query extractor/contract |
| JSON body | `Json<T>` | `Json::<T>::from_request(&req).await` |
| Form body | `Form<T>` | `Form::<T>::from_request(&req).await` |
| Response | `HttpResponse` / `IntoResponse` | `Response::json`, `html`, `text`, `bytes`, `redirect` |
| Middleware | `Transform` / `Layer` | Tower `Layer` with `ServiceBuilder` |
| Error type | `ResponseError` / `IntoResponse` | `DjangorsError` or app error `impl From<AppError>` |
| ORM | Diesel, SeaORM, SQLx, custom | `#[derive(Model)]`, `QuerySet`, `q!`, `Database` |
| Serialization | Serde structs | Serde contracts or `djangors-rest` serializers |
| Auth | custom extractor/middleware | `djangors-auth`, sessions, token/JWT auth, permissions |
| CRUD API | handwritten handlers or macros | REST `ViewSet` and `ModelSerializer` |
| HTML templates | Tera, Askama, Maud | `djangors-template` and class-based views |
| Jobs | Tokio task, external queue | `#[task]`, database queue, `Worker`, recurring jobs |
| Tests | `actix_web::test`, `tower::ServiceExt` | `djangors-test::TestClient`, `TestDatabase`, real-socket tests |
| Admin | build it yourself | `djangors-admin` |
| CLI | binary/subcommands you create | `dj new`, `run`, `migrate`, `test`, `runworker`, and more |

The most important architectural translation is this:

```text
Axum/Actix handler -> Djangors view
extractor           -> request extraction + contract
application service -> Djangors service
SQLx/Diesel query   -> repository using QuerySet
IntoResponse        -> DjangorsError + Response
Router::nest        -> Router::mount
Tower layer         -> Tower layer around RouterService
```

## Start with a familiar Rust layout

The smallest Djangors app can use `models.rs`, `views.rs`, and `urls.rs`. A
feature that has business rules should use a domain app:

```text
src/
├── main.rs
├── lib.rs
├── config.rs
├── runtime.rs             # process-global handles needed outside requests
├── error_renderer.rs      # optional project-wide response renderer
├── urls.rs                # root routes and shared mounts
├── apps.rs                # app registry / route composition
├── tasks.rs               # task modules and public queue helpers
└── apps/
    └── books/
        ├── mod.rs
        ├── models.rs
        ├── contracts.rs
        ├── errors.rs
        ├── repositories.rs
        ├── services.rs
        ├── permissions.rs
        ├── serializers.rs
        ├── views.rs
        ├── urls.rs
        ├── tasks.rs       # only when jobs belong specifically to books
        └── admin.rs

tests/apps/books/
├── models.rs
├── services.rs
├── permissions.rs
├── api.rs
└── tasks.rs
```

This is not an arbitrary folder convention. It gives each kind of code an
obvious home and keeps handlers from becoming giant Axum-style “everything
functions.” The school-management backend uses this shape across its domain
apps.

## Handlers and extractors

### Axum

An Axum handler commonly looks like this:

```rust,illustrative
async fn get_book(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<BookQuery>,
    Json(input): Json<UpdateBook>,
) -> Result<Json<BookResponse>, AppError> {
    // ...
}
```

### Actix Web

The same idea in Actix often uses extractors:

```rust,illustrative
async fn get_book(
    state: web::Data<AppState>,
    path: web::Path<BookPath>,
    query: web::Query<BookQuery>,
    body: web::Json<UpdateBook>,
) -> Result<impl Responder, AppError> {
    // ...
}
```

### Djangors

Djangors gives the handler a stable framework boundary. Extract values inside
the view and keep the extracted objects typed:

```rust,illustrative
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_core::extract::{FromRequest, Json};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct UpdateBook {
    pub title: String,
}

pub async fn update(
    req: Request,
    params: PathParams,
) -> Result<Response, DjangorsError> {
    let book_id: i64 = params.get_as("id")?;
    let Json(input) = Json::<UpdateBook>::from_request(&req).await?;
    let db = req.require_state::<djangors_db::Database>()?;

    // Call a service with `db`, `book_id`, and `input`.
    let _ = (db, book_id, input);
    Response::json(StatusCode::OK, &serde_json::json!({ "ok": true }))
}
```

`Request::require_state` gives a useful error when startup forgot to attach a
dependency. Prefer it over `state(...).ok_or_else(...)` in application code.

The request has two different storage areas:

- `req.state::<T>()` / `require_state::<T>()`: application-wide shared state
  attached by the router, such as a database pool, Redis client, settings, or
  mail backend.
- `req.ext::<T>()`: request-scoped extensions installed by middleware, such as
  a session, authenticated principal, tenant, request ID, or CSRF token.

Do not put a request-specific user or tenant into global router state.

## Routes and nesting

### Axum and Actix to Djangors

```rust,illustrative
use djangors_core::Router;
use crate::apps::{books, accounts};

pub fn urls() -> Router {
    Router::new()
        .get("/healthz", crate::views::healthz)
        .mount("/api/v1", books::urls())
        .mount("/accounts", accounts::urls())
}
```

`Router::mount` is the Djangors equivalent of Axum’s `nest` and Actix’s
`service(web::scope(...))`. The app router should only declare the app’s own
paths; the root router owns global prefixes and cross-cutting routes.

Typed path parameters use `{name:type}`:

```rust,illustrative
Router::new()
    .get("/books/{id:i64}", books::views::detail)
    .get("/books/{slug:slug}", books::views::by_slug);
```

The route parameter name must match `params.get_as("id")` in the view. Djangors
also supports named routes and reversal:

```rust,illustrative
let router = Router::new()
    .get("/books/{id:i64}", books::views::detail)
    .name("book-detail");
let path = router.reverse("book-detail", &[("id", "42")])?;
```

When route reversal is not available in a small isolated example, a formatted
path is acceptable. In a larger application, prefer route names so redirects
do not break when a prefix changes.

## State and dependency injection

In Axum, you usually define one `AppState` struct. In Actix, you often register
several `web::Data<T>` values. Djangors supports both styles:

```rust,illustrative
#[derive(Clone)]
pub struct AppState {
    pub settings: AppSettings,
    pub db: djangors_db::Database,
}

let state = AppState { settings, db };
let router = crate::urls::urls().with_state(state);
```

Or attach independent values:

```rust,illustrative
let router = crate::urls::urls()
    .with_state(db)
    .with_state(redis_client)
    .with_state(settings)
    .with_state(mail_backend);
```

Use a state struct when dependencies belong together and independent state
when an app should request only the capability it needs. Avoid a global
“service locator” containing every dependency; keep the explicit type lookup
close to the handler boundary and pass required values into services.

## Middleware and Tower

Actix middleware and Axum middleware are both conceptually Tower layers in
Djangors. The Djangors `Router` is the inner routing service; `RouterService`
adapts it to Hyper/Tower:

```rust,illustrative
let router_service = djangors_core::router::RouterService::new(router, debug);

let service = tower::ServiceBuilder::new()
    .layer(djangors_core::middleware::request_id_layer())
    .layer(djangors_core::middleware::security_headers_layer())
    .layer(djangors_sessions::SessionLayer::new(session_store))
    .layer(djangors_core::middleware::csrf_layer())
    .service(router_service);
```

Middleware ordering matters. A practical order is:

1. tracing/request ID
2. host validation and security headers
3. session/authentication state
4. tenant resolution
5. CSRF for cookie-authenticated unsafe requests
6. router dispatch and error rendering

Follow the specific layer’s documentation when composing a custom stack. Do
not assume a middleware runs before another just because its call appears
first; `ServiceBuilder` wrapping follows Tower’s layer semantics.

Use middleware for cross-cutting request policy: authentication context,
tenant resolution, request IDs, CSRF, headers, compression, and logging. Use a
permission object or service for endpoint/domain authorization.

## Errors: the equivalent of `ResponseError` / `IntoResponse`

Actix developers often implement `ResponseError`; Axum developers implement
`IntoResponse`. Djangors uses an app error enum plus one conversion:

```rust,illustrative
#[derive(Debug)]
pub enum BookError {
    NotFound,
    Forbidden,
    InvalidTitle,
    Database(String),
}

impl From<BookError> for djangors_core::DjangorsError {
    fn from(error: BookError) -> Self {
        use hyper::StatusCode;
        match error {
            BookError::NotFound => djangors_core::DjangorsError::api(
                StatusCode::NOT_FOUND, "book_not_found", "Book was not found."),
            BookError::Forbidden => djangors_core::DjangorsError::api(
                StatusCode::FORBIDDEN, "permission_denied", "You cannot edit this book."),
            BookError::InvalidTitle => djangors_core::DjangorsError::api(
                StatusCode::BAD_REQUEST, "invalid_title", "Title is required."),
            BookError::Database(message) => {
                tracing::error!(error = %message, "book database failure");
                djangors_core::DjangorsError::Internal("Book operation failed.".into())
            }
        }
    }
}
```

Views then stay readable:

```rust,illustrative
let book = crate::apps::books::services::find_for_user(db, user, id)
    .await
    .map_err(djangors_core::DjangorsError::from)?;
```

The router renders `DjangorsError`. By default, API errors use the JSON
envelope and ordinary errors can render HTML in a browser or JSON when the
client requests JSON. A project can register an `ErrorRenderer` in router
state to enforce a single envelope across HTML and API handlers.

Recommended envelope:

```json
{
  "error": {
    "code": "book_not_found",
    "message": "Book was not found.",
    "details": null
  }
}
```

Keep SQL, stack traces, credentials, and provider responses in logs, not
client responses. Add field-level details for validation errors. Do not use
Rust `Debug` formatting as an API message.

## Database access: from SQLx/Diesel to ORM repositories

The direct translation of an Axum handler that executes SQL is tempting, but
the scalable Djangors pattern is repository + service:

```rust,illustrative
// repositories.rs: database access only
pub async fn find(db: &djangors_db::Database, id: i64)
    -> Result<Option<Book>, djangors_orm::OrmError>
{
    use djangors_orm::{q, Model};
    Book::objects().filter(q!(id = id))?.first(db).await
}

// services.rs: domain policy
pub async fn find_for_user(
    db: &djangors_db::Database,
    user_id: i64,
    id: i64,
) -> Result<Book, BookError> {
    repositories::find_owned(db, user_id, id)
        .await
        .map_err(|error| BookError::Database(error.to_string()))?
        .ok_or(BookError::NotFound)
}
```

The repository owns `QuerySet`, `q!`, filters, ordering, joins, and persistence.
It does not return HTTP responses or inspect roles. The service owns
cross-record validation, transactions, state transitions, and coordination.
The view extracts HTTP input and maps the service result.

When raw SQL is necessary, isolate it in the repository and document why the
ORM cannot express it. Do not create a second ORM in every handler.

## JSON contracts, validation, and serializers

Your Axum `Json<T>` or Actix `web::Json<T>` type becomes a Djangors contract:

```rust,illustrative
#[derive(Debug, serde::Deserialize)]
pub struct CreateBook {
    pub title: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BookResponse {
    pub id: i64,
    pub title: String,
}
```

Do not deserialize directly into a persistence model when the client should
not control fields such as `id`, `owner_id`, `created_at`, or `status`.

For REST CRUD, use `ModelSerializer` and `FieldSet`:

```rust,illustrative
let serializer = djangors_rest::ModelSerializer::<Book>::new(
    djangors_rest::FieldSet::all()
        .read_only(&["id", "created_at"])
        .excluding(&["internal_notes"]),
);
```

Use `read_only` for server-owned values, `write_only` for passwords/tokens,
and custom serializers when the public API differs from the model. Use a
service for validation involving more than one record.

## CRUD APIs: ViewSets vs handwritten handlers

If an endpoint is ordinary authenticated list/create/retrieve/update/delete,
use the REST ViewSet helpers. They give you consistent serialization,
pagination, filters, permissions, and OpenAPI integration:

```rust,illustrative
use djangors_core::Router;
use djangors_rest::{viewset_routes_with_options, IsAuthenticated, ViewSetConfig, ViewSetOptions};

let options = ViewSetOptions::<Book>::new(ViewSetConfig {
    filterable_fields: &["published"],
    orderable_fields: &["title"],
    ..Default::default()
});
let router = viewset_routes_with_options::<Book, _>(
    Router::new(), "/books", options, IsAuthenticated);
```

Use a handwritten view for workflow actions (`publish`, `checkout`,
`approve`), unusual response shapes, file uploads, or behavior that does not
fit CRUD. The workflow should still call a service.

For tenant-owned records, use `Scoped` plus
`scoped_viewset_routes_with_config`. Scoping answers which rows are visible;
it does not automatically decide which role may edit them.

## Authentication, permissions, and tenant context

Actix/Axum users often put an authenticated user in an extractor. In Djangors,
session auth and API auth are framework facilities, and `current_user(&req)`
resolves the configured identity sources. `IsAuthenticated` is the route
guard; `IsStaff`, `IsSuperuser`, and composed permissions handle common role
rules.

Keep three decisions separate:

```text
authentication:  who is the caller?
permission:      may the caller perform this action?
scope:           which rows belong to the caller's tenant/owner?
```

For a hand-written endpoint, resolve the principal, resolve tenant context,
check the role, and query by tenant plus record ID. Validate every foreign key
against the same tenant before writing. Never trust a tenant ID supplied by a
client body.

## Background work: Tokio spawn vs Djangors tasks

`tokio::spawn` is appropriate for short-lived in-process work tied to the
server process. It is not a durable queue: work can disappear on restart,
there is no retry state, and a request may outlive its useful context.

For durable jobs use `djangors_tasks`:

```rust,illustrative
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SendBookReport {
    pub report_id: i64,
}

#[djangors_tasks::task]
pub async fn send_book_report(args: SendBookReport)
    -> Result<(), djangors_tasks::TaskError>
{
    let db = crate::runtime::db();
    // Reload by ID, perform idempotent work, persist status.
    let _ = (db, args);
    Ok(())
}

// Called by a service after committing the record the job needs.
djangors_tasks::enqueue(db, "send_book_report", &SendBookReport { report_id }).await?;
```

Task payloads must be serializable and small. Pass IDs, not request objects or
stale model graphs. Handlers have no request context, so install process-global
handles such as the database or mail backend once during startup, or pass
everything needed through the payload.

The worker records `pending`, `running`, `completed`, and `failed` states,
retries failures up to `max_attempts`, and isolates handler panics. Make every
task idempotent and treat an already-completed job as success. Use
`register_recurring` for cron-style jobs and start the worker only after task
tables and task registrations are ready.

## Templates and forms

For an HTML application:

- `djangors_template::TemplateEngine` is the equivalent of Tera/Askama’s
  template engine.
- `djangors_template::render` returns an HTTP response from a template.
- `Form<T>` parses URL-encoded forms into a typed Rust struct.
- `#[derive(Model)]` generates model-form metadata for the generic HTML views.
- `djangors_views` provides `ListView`, `DetailView`, `CreateView`,
  `UpdateView`, and `DeleteView` for conventional CRUD pages.

Keep the same boundary as an API: the view extracts form input, the service
applies business rules, and the template receives a safe response context.
Never interpolate unescaped user input into HTML.

## Files, uploads, and streaming

Use the framework’s multipart extraction and static-file/storage facilities
for uploads. A safe upload flow is:

1. Validate authenticated ownership and intended purpose.
2. Enforce size and content-type limits.
3. Store using a generated storage key, never a client filename as a path.
4. Persist metadata and scan untrusted content when required.
5. Queue expensive processing as a task.
6. Serve through a controlled storage/download endpoint.

For server push, Djangors provides SSE helpers (`Response::sse` and
`Router::get_sse`). Treat a streaming response as a separate handler kind; do
not try to return a normal `Response` from an SSE stream. Djangors is not a
drop-in WebSocket server—verify the framework capability before designing a
WebSocket protocol around it.

## Testing: translating Actix and Axum test habits

The Djangors equivalent of an in-process router test is `TestClient`:

```rust,illustrative
#[tokio::test]
async fn book_detail_returns_not_found() {
    let router = crate::urls::urls();
    let response = djangors_test::TestClient::new(router)
        .get("/api/v1/books/999")
        .send()
        .await;
    response.assert_status(hyper::StatusCode::NOT_FOUND);
}
```

Use `TestDatabase` for isolated database tests. Run the fast SQLite suite with
`TEST_BACKEND=sqlite` and run PostgreSQL-specific behavior against PostgreSQL
as well. SQLite is useful for speed but cannot prove PostgreSQL locking,
extensions, index behavior, or every SQL expression.

Keep tests outside production modules and group them by app:

```text
tests/apps/books/
├── models.rs       # metadata and database constraints
├── services.rs     # domain rules without HTTP
├── permissions.rs  # role and tenant boundaries
├── api.rs          # status, headers, JSON/form contracts
└── tasks.rs        # payload, retry, idempotency behavior
```

For each endpoint test success, anonymous access, wrong role, missing records,
malformed input, cross-tenant IDs, conflicts, and the exact error envelope.

## Configuration, startup, and deployment

Actix/Axum applications often build an `AppState` directly in `main`. Djangors
startup should additionally initialize framework services:

1. Load `DjangorsSettings` and typed application settings.
2. Initialize dev or production `tracing`.
3. Connect the database and run migrations.
4. Create task tables and install runtime handles.
5. Build Redis, mail, cache, and external provider clients.
6. Register recurring jobs and start the worker.
7. Build the root router and attach typed state.
8. Wrap `RouterService` in sessions, tenant, security, CSRF, and logging layers.
9. Run the service with graceful shutdown.

Expose `/healthz` for process liveness and `/readyz` for dependency readiness.
Readiness failures should return `503` and safe generic messages while logging
the driver error. Use `dj check --deploy`, `cargo build --release`, and the
deployment guide before shipping.

## The migration checklist

When moving one Actix/Axum feature into Djangors, do it in this order:

1. Write down its routes, methods, path/query/body contracts, status codes,
   auth rules, tenant rules, and error envelope.
2. Move persistent structs to `models.rs` and create a migration.
3. Define public input/output structs in `contracts.rs`.
4. Move SQL into `repositories.rs`.
5. Move validation and workflows into `services.rs`.
6. Define domain failures in `errors.rs` and map them to `DjangorsError`.
7. Add serializers or ViewSet options for public field visibility.
8. Implement thin views and app-local URLs.
9. Add permission/scoping checks before enabling routes.
10. Add service, permission, model, API, and task tests.
11. Attach dependencies in startup and verify middleware order.
12. Run both SQLite and PostgreSQL validation where the feature uses
    PostgreSQL-specific behavior.

If you can point to the file that owns each item, the feature is usually easy
to maintain. If a view owns SQL, business rules, error formatting, and task
enqueueing all at once, split it before migrating the next feature.
