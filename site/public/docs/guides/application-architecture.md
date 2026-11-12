# Application Architecture

This guide explains how to organize a real Djangors application after you
finish the polls tutorial. The framework does not force one architecture, but
the following shape is a good default for beginners and scales well as an app
grows.

The short version is:

```text
URL -> view -> contract / permission -> service -> repository -> ORM / database
```

Each layer has one job. Keeping those jobs separate makes code easier to find,
test, and change.

## Which path should I follow?

- **New to web development:** follow the [8-part tutorial](../tutorial/01-requests-and-responses.md), then return here and build one small feature using the `books` examples.
- **Coming from Django:** read the [Django comparison](../django-comparison.md), then use this guide for the production app boundaries that Django’s conventions often leave implicit.
- **Coming from Actix Web or Axum:** read [Moving from Actix Web or Axum](../migration/from-actix-axum.md) for handler, state, router, middleware, error, and testing translations.

Do not create every layer on the first five-line endpoint. Begin with a view
and route, add a model when data is persistent, and split contracts,
repositories, services, permissions, and errors as soon as the feature has
more than one workflow or endpoint.

## The recommended app layout

For a small project, start with `src/models.rs`, `src/views.rs`, and
`src/urls.rs`. Once a feature has more than one endpoint or meaningful business
rules, give it a domain app:

```text
src/
├── main.rs                 # process startup and middleware
├── lib.rs                  # public modules
├── urls.rs                 # root router; mounts app routers
└── apps/
    └── books/
        ├── mod.rs          # app module and public API
        ├── models.rs       # database records
        ├── contracts.rs    # request and response DTOs
        ├── repositories.rs # database queries
        ├── services.rs     # business rules and workflows
        ├── views.rs        # HTTP handlers
        ├── urls.rs         # app-local routes
        ├── serializers.rs  # API representation, when using REST
        ├── permissions.rs  # access rules, when needed
        └── admin.rs        # admin registration, when needed

tests/
└── apps/books/
    ├── models.rs
    ├── services.rs
    ├── permissions.rs
    └── api.rs
```

The reference school-management backend uses this same domain-app pattern for
each feature. Copy the boundaries, not the domain names.

## What each file means

### `models.rs`: what is stored

A model is a typed database record. It describes fields, relations, and schema
metadata. It may contain small, record-specific methods such as
`is_archived()`, but it should not know about HTTP requests or decide who is
allowed to call an endpoint.

```rust,illustrative
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(app = "books", table_name = "books_book")]
pub struct Book {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 200)]
    pub title: String,
    #[djangors(default = false)]
    pub published: bool,
}

impl Book {
    pub fn can_be_checked_out(&self) -> bool {
        self.published
    }
}
```

Use `ForeignKey<T>` for native Djangors relationships and keep database
details here. Put input validation that spans multiple records in a service.

### `contracts.rs`: what crosses the boundary

A contract is a request, path/query object, or response shape. It is also
called a DTO (data-transfer object). Contracts prevent database columns from
accidentally becoming your public API.

```rust,illustrative
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateBook {
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct BookResponse {
    pub id: i64,
    pub title: String,
}
```

Use `Option<T>` for fields in a partial update. A response contract should
contain only data clients are allowed to see. If the API calls an identifier
`public_id`, do not expose an internal database key just because the model has
one.

### `repositories.rs`: how data is read and written

A repository owns ORM and database access. It returns models or domain-level
data; it never returns `Response`, reads headers, or checks a user's role.

```rust,illustrative
use djangors_db::Database;
use djangors_orm::{q, Model};

use super::models::Book;

pub async fn find_by_id(db: &Database, id: i64)
    -> Result<Option<Book>, djangors_orm::OrmError>
{
    Book::objects().filter(q!(id = id))?.first(db).await
}

pub async fn published(db: &Database)
    -> Result<Vec<Book>, djangors_orm::OrmError>
{
    Book::objects().filter(q!(published = true))?.all(db).await
}
```

Keep query construction here so a change from one query to another has one
obvious home. Always include tenant or owner filters in repository queries for
tenant-scoped data; do not rely on every view remembering them.

### `services.rs`: what the application is allowed to do

A service owns business rules and workflows: validation, state transitions,
cross-record checks, transactions, and coordination between repositories. It
should be callable from a view, a background task, a management command, or a
test.

```rust,illustrative
use super::{contracts::CreateBook, models::Book};

#[derive(Debug, PartialEq, Eq)]
pub enum CreateBookError {
    EmptyTitle,
}

pub fn validate_new_book(input: &CreateBook) -> Result<(), CreateBookError> {
    if input.title.trim().is_empty() {
        return Err(CreateBookError::EmptyTitle);
    }
    Ok(())
}

pub fn new_book(input: CreateBook) -> Result<Book, CreateBookError> {
    validate_new_book(&input)?;
    Ok(Book { id: 0, title: input.title.trim().to_owned(), published: false })
}
```

The example separates validation from persistence so it is easy to test. A
real create service would validate, construct the model, save it through a
repository, and return the saved record.

### `views.rs`: translate HTTP into application calls

A view is an adapter. It extracts request data, calls permissions and services,
then maps the result to an HTTP response. It should not contain SQL or a
multi-step business workflow.

```rust,illustrative
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_core::extract::{FromRequest, Json};

use super::{contracts::CreateBook, services};

pub async fn create(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let Json(input) = Json::<CreateBook>::from_request(&req).await?;
    services::validate_new_book(&input)
        .map_err(|_| DjangorsError::api(StatusCode::BAD_REQUEST, "invalid_book", "title is required"))?;
    // Call the create service here, then serialize its returned Book.
    Ok(Response::json(StatusCode::CREATED, &serde_json::json!({
        "title": input.title
    }))?)
}
```

For production code, use a single error-mapping policy. Domain errors should
be converted to useful client messages and status codes; do not send Rust
`Debug` output such as `InvalidStatus` to API consumers.

### `urls.rs`: which HTTP request reaches which view

The app router only declares paths, methods, and handlers. It does not query
the database or implement authorization.

```rust,illustrative
use djangors_core::Router;
use super::views;

pub fn urls() -> Router {
    Router::new()
        .get("/books", views::list)
        .post("/books", views::create)
        .get("/books/{id:i64}", views::detail)
}
```

Mount app routers once from the root router:

```rust,illustrative
pub fn urls() -> djangors_core::Router {
    djangors_core::Router::new()
        .mount("/api/v1", apps::books::urls())
}
```

The route parameter name (`id`) must match the name read by the view, and its
type (`i64`) determines how the router parses the value.

## The request flow in one sentence

For `POST /api/v1/books`, `urls.rs` selects `views::create`; the view parses a
`CreateBook` contract, checks permissions, calls `services::create_book`; the
service applies business rules and calls `repositories::insert`; the view
serializes the result as a `BookResponse`.

## Errors: define them once, render them consistently

Production applications should not scatter `DjangorsError::api(...)` calls
through every branch of every view. Give each domain app an `errors.rs` file.
The domain error describes what went wrong; its conversion to
`DjangorsError` describes how HTTP clients should see it.

```rust,illustrative
use djangors_core::DjangorsError;
use hyper::StatusCode;

#[derive(Debug)]
pub enum BookError {
    Unauthenticated,
    Forbidden,
    NotFound,
    EmptyTitle,
    DuplicateTitle,
    Persistence(String),
}

impl From<BookError> for DjangorsError {
    fn from(error: BookError) -> Self {
        match error {
            BookError::Unauthenticated => DjangorsError::api(
                StatusCode::UNAUTHORIZED,
                "not_authenticated",
                "Authentication credentials were not provided.",
            ),
            BookError::Forbidden => DjangorsError::api(
                StatusCode::FORBIDDEN,
                "permission_denied",
                "You do not have permission to perform this action.",
            ),
            BookError::NotFound => DjangorsError::api(
                StatusCode::NOT_FOUND,
                "book_not_found",
                "Book was not found.",
            ),
            BookError::EmptyTitle => DjangorsError::api(
                StatusCode::BAD_REQUEST,
                "empty_title",
                "Title is required.",
            ),
            BookError::DuplicateTitle => DjangorsError::api(
                StatusCode::CONFLICT,
                "duplicate_title",
                "A book with that title already exists.",
            ),
            // Log the detailed database error server-side, but do not expose
            // connection strings, SQL, or driver details to the client.
            BookError::Persistence(message) => {
                tracing::error!(error = %message, "book persistence failure");
                DjangorsError::Internal("Could not save the book.".to_owned())
            }
        }
    }
}
```

The normal mapping is:

| Failure | HTTP status | Stable code |
| --- | ---: | --- |
| Missing/invalid authentication | `401` | `not_authenticated` |
| Authenticated but not allowed | `403` | `permission_denied` |
| Invalid input | `400` or `422` | feature-specific code |
| Record not found | `404` | `<resource>_not_found` |
| Conflict with existing state | `409` | feature-specific code |
| Unexpected persistence/infrastructure failure | `500` | generic internal error |

Use `Display` for errors that must appear inside a list or job report. Never
use `format!("{error:?}")` in a client response: that exposes Rust variant
names and may leak implementation details.

### Validation details

For several invalid fields, attach structured details instead of returning
only one sentence. The client can then highlight the right inputs:

```rust,illustrative
use serde_json::json;
use hyper::StatusCode;

let error = DjangorsError::api(StatusCode::BAD_REQUEST, "invalid", "Request failed.")
    .with_details(json!({
        "errors": [
            { "field": "title", "code": "blank", "message": "must not be blank" },
            { "field": "isbn", "code": "invalid_format", "message": "must be 13 digits" }
        ]
    }));
```

### Error rendering

Handlers return `Result<Response, DjangorsError>`; they do not render every
error themselves. The router renders an error after a handler returns `Err`.
By default, an API error becomes JSON, while non-API errors can become HTML or
JSON based on the request's `Accept` header. In debug mode the HTML response
can contain a rich diagnostic page; production should return a minimal generic
page and log the real cause.

If an application needs one response shape for every endpoint, implement
`djangors_core::error::ErrorRenderer` and register it as router state:

```rust,illustrative
use djangors_core::error::ErrorRenderer;
use djangors_core::{DjangorsError, Request, Response};
use serde_json::json;

pub struct ApiErrorRenderer;

impl ErrorRenderer for ApiErrorRenderer {
    fn render(&self, error: &DjangorsError, _request: &Request) -> Response {
        Response::json(error.status_code(), &json!({
            "error": {
                "code": error.code(),
                "message": error.message(),
                "details": error.details().cloned().unwrap_or(serde_json::Value::Null)
            }
        })).expect("error envelope is serializable")
    }
}
```

Register an `Arc<dyn ErrorRenderer>` with `.with_state(...)` when constructing
the router. A project-wide renderer is useful when matching an existing API
contract, for example:

```json
{
  "error": {
    "code": "book_not_found",
    "message": "Book was not found.",
    "details": null
  }
}
```

Do not put database URLs, SQL, stack traces, file paths, usernames, or provider
responses in production error bodies. Log those details with `tracing` and
return a safe message. Health and readiness endpoints follow the same rule:
return a generic `database unreachable` message while logging the driver error.

## Serializers: control API representation

Contracts describe shapes; serializers decide how model values are represented
and which fields can be written. For CRUD APIs, prefer the REST framework's
`ModelSerializer` and a narrow `FieldSet` rather than returning a model
directly:

```rust,illustrative
use djangors_rest::{FieldSet, ModelSerializer};
use crate::apps::books::models::Book;

let serializer = ModelSerializer::<Book>::new(
    FieldSet::all()
        .read_only(&["id"])
        .excluding(&["internal_notes"]),
);
```

Mark IDs, timestamps, ownership, and computed fields read-only. Mark secrets,
passwords, and tokens write-only. Use a custom serializer when the wire shape
does not match database fields. A serializer is not a replacement for a
service: field syntax validation belongs there, while cross-record rules and
state transitions belong in services.

## Permissions and tenant isolation

Authentication answers “who is this?”; permission answers “may this user do
this?”; scoping answers “which rows may this user see?” These are separate
checks.

For a school- or tenant-scoped model, implement `djangors_rest::Scoped` so
every ViewSet operation applies the scope. Then use
`scoped_viewset_routes_with_config` when you also need filters or ordering.
Add role checks for write actions—row scoping alone does not distinguish a
viewer from an administrator.

For hand-written handlers, the safe order is:

1. Resolve the authenticated principal.
2. Resolve the tenant from the principal/session, not from an unchecked body field.
3. Check the role or permission.
4. Query by both tenant and record ID.
5. Validate foreign keys against the same tenant before writing.

Never load a record by ID first and check its tenant later if a not-found and a
cross-tenant record should be indistinguishable. Scope the repository query so
the record is invisible outside its tenant.

## Background tasks

Tasks are for work that should not block an HTTP request: email, file scanning,
CSV imports, report generation, cleanup, and periodic billing. The school
backend keeps task modules at `src/tasks/` and, for feature-specific work, in
`src/apps/<app>/tasks.rs`.

### A task is a serializable command

Task handlers do not receive `Request`, sessions, or a database reference.
They receive a serializable payload and return `Result<(), TaskError>`:

```rust,illustrative
use djangors_tasks::{task, TaskError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateReportArgs {
    pub report_id: i64,
}

#[task]
pub async fn generate_report(args: GenerateReportArgs) -> Result<(), TaskError> {
    let db = crate::runtime::db();
    // Load the report by ID, perform the work, and persist its status.
    tracing::info!(report_id = args.report_id, "generating report");
    let _ = db;
    Ok(())
}
```

Pass IDs and small immutable values, not a model instance or a request. The
task should reload current state when it runs. This avoids stale serialized
records and keeps queue payloads small.

### Enqueue and run tasks

Enqueue from a service after the database state needed by the task exists:

```rust,illustrative
let task_id = djangors_tasks::enqueue(
    db,
    "generate_report",
    &GenerateReportArgs { report_id: report.id },
).await?;
```

Use `enqueue_scheduled` for delayed work. Start a worker after database setup:

```rust,illustrative
let worker = djangors_tasks::Worker::new(db.clone())
    .with_recurring_tick_interval(std::time::Duration::from_secs(60));
tokio::spawn(async move { worker.run().await; });
```

The queue claims tasks transactionally. PostgreSQL workers use row locking with
`FOR UPDATE SKIP LOCKED`, so multiple workers can run safely. A successful task
becomes `completed`; an error or panic is recorded and retried until
`max_attempts`, after which it becomes `failed`.

### Task best practices

- Make handlers idempotent: a retry must not send the same payment twice or create duplicate rows.
- Record a durable status (`pending`, `processing`, `completed`, `failed`) for long workflows.
- Treat “already completed” as success and return early.
- Log the task name and stable job ID, not sensitive payloads.
- Convert operational failures to `TaskError::TaskExecution`, preserving the cause in server logs.
- Handle expected missing data deliberately; skip and log cleanup items when safe, fail when the result is incomplete.
- Do not panic for ordinary validation or missing-record cases.
- Test the service logic directly and test at least one queued-task path with retries/status transitions.

### Recurring tasks

Register recurring jobs at startup with a standard five-field cron expression:

```rust,illustrative
djangors_tasks::register_recurring(
    db,
    "generate_report",
    &GenerateReportArgs { report_id: 42 },
    "0 2 * * *", // every day at 02:00
).await?;
```

Create the task and recurring-task tables before starting the worker. Anchor
task modules if the binary uses link-time registration, and make recurring-job
registration safe to run repeatedly. A restart should not create harmful
duplicates or execute the same business operation twice.

## Startup, shared state, and middleware

Keep `main.rs` as wiring, not business logic. A production startup sequence is:

1. Load framework settings and application settings.
2. Initialize development or production logging.
3. Connect the database and run migrations.
4. Create task tables and install process-global handles required by task handlers.
5. Build external clients such as Redis and mail backends.
6. Register recurring jobs and start the worker.
7. Build the root router and attach typed shared state with `.with_state(...)`.
8. Wrap the router in sessions, tenant resolution, security headers, CSRF, and other middleware.
9. Start the server with graceful shutdown.

```rust,illustrative
let router = crate::urls::urls()
    .with_state(db.clone())
    .with_state(redis_client)
    .with_state(app_settings.clone())
    .with_state(std::sync::Arc::new(ApiErrorRenderer)
        as std::sync::Arc<dyn djangors_core::error::ErrorRenderer>);

let service = tower::ServiceBuilder::new()
    .layer(djangors_sessions::SessionLayer::new(session_store))
    .layer(djangors_core::middleware::security_headers_layer())
    .service(djangors_core::router::RouterService::new(router, debug));
```

The root `urls.rs` should expose liveness/readiness endpoints, mount the app
router once (for example at `/api/v1`), and mount the admin site. Shared state
belongs in the router; request-specific values belong in request extensions.

## Feature-specific modules

The canonical files are boundaries, not a limit on the number of files an app
may have. The reference school-management backend adds focused modules when a
feature needs them:

| Module | Use it for | Examples |
| --- | --- | --- |
| `tasks.rs` | app-owned durable jobs | imports, notifications, cleanup |
| `notifications.rs` | email/SMS/push orchestration | trial notices, status updates |
| `emails.rs` / `email_templates.rs` | mail backend and message content | shared account mail |
| `storage.rs` | object/file storage adapter | uploads, download keys |
| `importers.rs` / `exporters.rs` | CSV or bulk data formats | student imports, reports |
| `pdf.rs` | document-specific PDF layout | certificates, receipts |
| `pagination.rs` | app-specific list response rules | legacy endpoint parity |
| `audit.rs` | durable audit events | actor, action, target, metadata |
| `crypto.rs` / `verification.rs` | cryptographic or verification workflows | tokens, enrollment |
| `moodle_client.rs` or another client module | external provider adapter | LMS or payment provider |

Keep these modules focused. An external client should not decide HTTP status
codes; a PDF renderer should not issue database queries; an email template
should not implement account authorization. Put orchestration in a service and
queue slow or retryable work through a task.

## Health and readiness endpoints

Keep these meanings distinct:

- `/healthz`: the process is alive; normally return `200` with `{"status":"ok"}`.
- `/readyz`: dependencies are usable; return `200` when all checks pass and `503` when one fails.

Build readiness JSON from a pure snapshot so it can be tested without a live
database or Redis connection. Log the underlying probe error, but return only
safe generic messages to callers.

## Migrations and app ownership

Each domain app owns its migration files. A migration should have explicit
forward and rollback sections, create constraints and indexes intentionally,
and be applied in foreign-key dependency order. Do not hide production schema
changes in a task or application startup side effect. Test important unique,
foreign-key, check, and tenant constraints.

## Testing the architecture

Keep production source free of test modules. Group tests by app:

```text
tests/apps/books/
├── models.rs       # metadata, constraints, relationships
├── services.rs     # validation and state transitions
├── permissions.rs  # roles, authentication, tenant isolation
└── api.rs          # real request/response status and JSON contract
```

For every endpoint, cover the happy path and the boundary cases: anonymous,
wrong role, missing record, malformed input, cross-tenant ID, database conflict,
and the exact response envelope. Test task services without a worker first,
then add focused worker tests for serialization, retries, and idempotency.

## Rules that keep projects maintainable

- Keep HTTP concerns in views: requests, path parameters, status codes, and responses.
- Keep SQL and `QuerySet` construction in repositories.
- Keep business rules in services and make them testable without HTTP.
- Keep public request and response shapes in contracts/serializers.
- Keep route declarations in `urls.rs`; use app-local routers and mount them at the root.
- Put integration tests under `tests/`, grouped by app; test services, permissions, models, and real API responses separately.
- For tenant-scoped records, apply the tenant filter on every read and write, and test cross-tenant access explicitly.
- Let apps call another app through its public service API instead of importing its repository internals.

When a feature is tiny, combining files is fine. Split it into these layers as
soon as duplication or business rules appear; architecture is a tool for
clarity, not ceremony.
