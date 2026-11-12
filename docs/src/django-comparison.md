# Djangors for Django Developers

This guide provides a direct side-by-side translation reference for experienced Django developers learning Djangors. While Djangors preserves Django's familiar mental model (models, views, routers, settings, admin, authentication, migrations), Rust's type system and asynchronous runtime introduce distinct structural and safety guarantees.

Use this guide for vocabulary and API translation. For production file
boundaries, read the [Application Architecture guide](guides/application-architecture.md).
For Actix Web or Axum users, see [Moving from Actix Web or Axum](migration/from-actix-axum.md).

---

## 1. Models and Schema Definition

In Django, models inherit from `models.Model` and use Python field descriptors. In Djangors, models are plain Rust `struct`s deriving `#[derive(Model)]` with `#[djangors(...)]` attributes.

### Django (`models.py`)

```python
from django.db import models

class Book(models.Model):
    title = models.CharField(max_length=200)
    author = models.ForeignKey('Author', on_delete=models.CASCADE)
    is_published = models.BooleanField(default=True)
    created_at = models.DateTimeField(auto_now_add=True)

    class Meta:
        db_table = "library_book"
        ordering = ["-created_at"]
```

### Djangors

```rust,compile
# use djangors_orm::Model;
# #[derive(djangors_macros::Model, Debug, Clone)]
# #[djangors(app = "library", table_name = "library_author")]
# pub struct Author { #[djangors(primary_key, auto)] pub id: i64, #[djangors(max_length = 200)] pub name: String }
use djangors_macros::Model;
use djangors_orm::ForeignKey;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone)]
#[djangors(app = "library", table_name = "library_book", ordering = "-created_at")]
pub struct Book {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200)]
    pub title: String,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "books"))]
    pub author: ForeignKey<Author>,

    #[djangors(default = true)]
    pub is_published: bool,

    pub created_at: DateTime<Utc>,
}
```

### Key Differences & Guarantees
- **Explicit Primary Key Required**: In Djangors, primary key fields (such as `id: i64` with `#[djangors(primary_key, auto)]`) must be explicitly declared on every model.
- **Compile-Time Metadata**: `#[derive(Model)]` generates model metadata (`Model::meta()`) at compile time. Field types and relation targets are checked by `rustc`.

---

## 2. URL Routing

Django uses list-based `urlpatterns` with `path()` or `re_path()`. Djangors uses builder-pattern `Router` instances with method-chained route registrations (`.get()`, `.post()`, `.route()`).

### Django (`urls.py`)

```python
from django.urls import path
from . import views

urlpatterns = [
    path('articles/', views.list_articles, name='article-list'),
    path('articles/<int:id>/', views.article_detail, name='article-detail'),
]
```

### Djangors

```rust,compile
# fn main() {
# use djangors_core::{Request, PathParams, Response, DjangorsError, StatusCode};
# async fn list_articles(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::text(StatusCode::OK, "")) }
# async fn article_detail(_: Request, _: PathParams) -> Result<Response, DjangorsError> { Ok(Response::text(StatusCode::OK, "")) }
use djangors_core::Router;

let router = Router::new()
    .get("/articles/", list_articles)
    .get("/articles/{id}", article_detail);
# }
```

### Key Differences & Guarantees
- **Path Parameters**: Djangors routes use `{id}` syntax for path parameters rather than `<int:id>`.
- **Method Scope**: Routes are bound directly to HTTP verbs (`.get()`, `.post()`, `.put()`, `.delete()`).

---

## 3. View Handlers

Django views receive an `HttpRequest` object and return an `HttpResponse`. Djangors view handlers are `async fn` functions receiving `Request` and `PathParams`, returning `Result<Response, DjangorsError>`.

### Django (`views.py`)

```python
from django.http import HttpResponse, Http404
from django.shortcuts import render
from .models import Article

def article_detail(request, id):
    try:
        article = Article.objects.get(pk=id)
    except Article.DoesNotExist:
        raise Http404("Article not found")
    return render(request, "articles/detail.html", {"article": article})
```

### Djangors

```rust,compile
# #[derive(djangors_macros::Model, Debug, Clone, serde::Serialize)]
# #[djangors(app = "library", table_name = "library_article")]
# pub struct Article { #[djangors(primary_key, auto)] pub id: i64, pub title: String }
use djangors_orm::Model;
use djangors_core::{Request, Response, PathParams, DjangorsError, StatusCode};

// A process-wide template engine, mirroring djangors-admin's own `ADMIN_TEMPLATES` pattern.
static TEMPLATES: std::sync::LazyLock<djangors_template::TemplateEngine> =
    std::sync::LazyLock::new(|| {
        djangors_template::TemplateEngine::new(vec!["templates".into()]).unwrap()
    });

pub async fn article_detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let article_id: i64 = params.get_as("id")?;
    let db = req.state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("Database state missing".into()))?;

    let article = Article::objects()
            .filter(djangors_orm::q!(id = article_id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|_| DjangorsError::NotFound)?;

    djangors_template::render(&TEMPLATES, "articles/detail.html", minijinja::context! { article })
}
```

### Key Differences & Guarantees
- **Async by Default**: All Djangors view handlers are non-blocking `async fn` operations executing on the Tokio runtime.
- **Explicit Database State**: Request state (like `Database` connection pools) is explicitly retrieved via `req.state::<Database>()`.

### Generic Class-Based Views

For the common list/detail/create/update/delete shape, `djangors-views` mirrors Django's
`django.views.generic`. The hand-written `article_detail` above could instead be:

```rust,illustrative
use djangors_views::{DetailView, ViewSetConfig};

let config = ViewSetConfig { engine: &TEMPLATES, template_name: "articles/detail.html", success_url: "/articles/" };
DetailView::<Article>::detail(req, params, &config).await
```

`ListView`/`CreateView`/`UpdateView`/`DeleteView` follow the same shape, generic over any
`#[derive(Model)]` type (`CreateView`/`UpdateView` additionally need the model's own generated
`ModelForm` methods, see [§8](#8-form-handling--extraction)).

---

## 4. Command-Line Interface (`manage.py` vs `dj`)

Django uses `python manage.py <command>`. Djangors provides the `dj` command-line utility.

| Django Command | Djangors Command | Description |
| :--- | :--- | :--- |
| `python manage.py startproject` | `dj new` | Scaffolds a new project |
| `python manage.py startapp` | `dj new-app` | Scaffolds a new app/module inside a project |
| `python manage.py runserver` | `dj run` | Starts dev server with live-reloading file watch loop |
| `python manage.py migrate` | `dj migrate` | Applies pending database migrations |
| `python manage.py makemigrations` | `dj makemigrations` | Introspects the project binary and generates numbered schema migrations |
| `python manage.py sqlmigrate` | `dj sqlmigrate <app> <migration>` | Renders a migration's SQL without applying it |
| `python manage.py showmigrations` | `dj showmigrations` | Lists migrations and their applied state |
| `python manage.py createsuperuser` | `dj createsuperuser` | Prompts for superuser credentials and creates User |
| `python manage.py collectstatic` | `dj collectstatic` | Bundles static assets into a production output dir |
| `python manage.py runworker` | `dj runworker` | Starts the background-task worker loop |
| `python manage.py test` | `dj test` | Runs workspace unit and integration test suite (`cargo test`) |
| `python manage.py shell` | `dj shell` | Launches interactive Rust REPL via `evcxr` |
| `python manage.py dbshell` | `dj dbshell` | Connects directly to configured database CLI |
| `python manage.py check` | `dj check [--deploy]` | Runs static checks (with `--deploy`, production-safety checks) |
| `python manage.py createpermissions` | `dj createpermissions` | Generates the standard `view`/`add`/`change`/`delete` permissions per model |

### `dj makemigrations` scope
`dj` runs the project's own binary in a hidden model-introspection mode, so registrations from application crates are visible. It stores the last model state in `migrations/.schema_snapshot.json` and generates numbered SQL migrations. `detect_changes` automatically detects new tables (`CreateTable`), added columns (`AddColumn`), dropped columns (`DropColumn`), changed column types (`AlterColumnType`), `CreateIndex`/`DropIndex`, `ManyToMany` join tables, `OneToOne` unique constraints, and `choices` `CHECK` constraints. (Column renames are emitted as a `DropColumn` + `AddColumn` pair unless authored manually.)

Database migrations in Djangors are authored as raw SQL files or generated via programmatic schema utilities.

### ℹ️ `dj shell` (evcxr Rust REPL)
`dj shell` launches an interactive Rust REPL via `evcxr` (installed via `cargo install evcxr_repl`).

Because `dj` is a separate binary process from your application, target project models cannot be auto-imported across process boundaries automatically. To import your project's models into the REPL session, use `:dep` with a path dependency:
```rust,illustrative
:dep my_app = { path = "." }
use my_app::models::*;
```

---

## 5. Admin Site Registration

Django registers models with `admin.site.register(Model, ModelAdmin)`. Djangors uses `AdminSite::register_with::<M>(ModelAdminConfig)` for type-safe static configuration.

### Django (`admin.py`)

```python
from django.contrib import admin
from .models import Article

@admin.register(Article)
class ArticleAdmin(admin.ModelAdmin):
    list_display = ["title", "author", "is_published"]
    search_fields = ["title"]
    list_filter = ["is_published"]
```

### Djangors

```rust,compile
# #[derive(djangors_macros::Model, Debug, Clone)]
# #[djangors(app = "library", table_name = "library_article")]
# pub struct Article { #[djangors(primary_key, auto)] pub id: i64, pub title: String, pub author: String, pub is_published: bool }
# fn main() {
use djangors_admin::{AdminSite, ModelAdminConfig};

let site = AdminSite::new().with_site_header("Admin Console");
site.register_with::<Article>(ModelAdminConfig {
    list_display: Some(&["title", "author", "is_published"]),
    search_fields: Some(&["title"]),
    list_filter: Some(&["is_published"]),
    ..Default::default()
});
# }
```

### Key Differences & Guarantees
- **Compile-Time Field Validation**: Djangors validates that fields listed in `list_display`, `search_fields`, and `list_filter` actually exist on the target model struct at registration.
- **Search Type Safety**: `search_fields` enforces text-compatible fields, and `list_filter` enforces boolean fields.

---

## 6. Settings and Configuration

Django uses a Python file (`settings.py`). Djangors uses `DjangorsSettings`, which loads from `djangors.toml` and environment variable overrides (`DJANGORS_*`).

### Django (`settings.py`)

```python
DEBUG = True
ALLOWED_HOSTS = ["localhost", "127.0.0.1"]
SECRET_KEY = "django-insecure-secret"
```

### Djangors (`djangors.toml` & `DjangorsSettings`)

```toml
# djangors.toml
debug = true
allowed_hosts = ["localhost", "127.0.0.1"]
secret_key = "djangors-insecure-secret"
host = "127.0.0.1"
port = 8000
```

Environment variable overrides:
```bash
export DJANGORS_DEBUG=true
export DJANGORS_ALLOWED_HOSTS="localhost,127.0.0.1"
export DJANGORS_SECRET_KEY="supersecret"
export DJANGORS_PORT=8000
```

Loading in Rust:
```rust,compile
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use djangors_core::DjangorsSettings;

let (settings, warnings) = DjangorsSettings::load()?;
# Ok(())
# }
```

---

## 7. CSRF Protection

Django uses `{% csrf_token %}` tag and `CsrfViewMiddleware` to check both request headers and POST form bodies. Djangors provides `csrf_layer()`.

### Header-First with Form-Body Fallback
1. **Cookie**: CSRF token is stored in the `csrftoken` cookie.
2. **Header Check First**: `CsrfLayer` checks for the token in the `X-CSRFToken` HTTP header on unsafe requests (POST, PUT, PATCH, DELETE).
3. **Form Body Fallback**: If the HTTP header is absent, Djangors checks `application/x-www-form-urlencoded` POST request bodies for a `csrfmiddlewaretoken` field.

In HTML forms:
```html
<form method="post">
  <input type="hidden" name="csrfmiddlewaretoken" value="{{ csrf_token }}">
  <button type="submit">Submit</button>
</form>
```

Adding the CSRF layer works like every Djangors middleware: it's composed via
`tower::ServiceBuilder` around a `RouterService`, not a method on `Router` itself:
```rust,compile
# fn main() {
# let router = djangors_core::Router::new();
# let settings = djangors_core::DjangorsSettings::default();
use djangors_core::middleware::csrf_layer;
use djangors_core::router::RouterService;
use tower::ServiceBuilder;

let router_service = RouterService::new(router, settings.debug);
let service = ServiceBuilder::new()
    .layer(csrf_layer())
    .service(router_service);
# }
```

---

## 8. Form Handling & Extraction

Django provides `forms.Form` and `forms.ModelForm` for rendering and parsing HTML forms. Djangors uses the `Form<T>` extractor to deserialize form bodies directly into Rust structs.

### Django (`forms.py` / `views.py`)

```python
class ContactForm(forms.Form):
    name = forms.CharField(max_length=100)
    email = forms.EmailField()

def contact_view(request):
    form = ContactForm(request.POST)
    if form.is_valid():
        name = form.cleaned_data['name']
```

### Djangors

```rust,compile
use serde::Deserialize;
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{Request, Response, PathParams, DjangorsError, StatusCode};

#[derive(Debug, Deserialize)]
pub struct ContactForm {
    pub name: String,
    pub email: String,
}

pub async fn contact_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let Form(form) = Form::<ContactForm>::from_request(&req).await?;
    println!("Submitted name: {}", form.name);
    Ok(Response::text(StatusCode::OK, "OK"))
}
```

### `ModelForm` Parity

`#[derive(Model)]` also generates a `ModelForm` equivalent directly on the model itself, so no
second, hand-written form struct is required:

```rust,compile
# use djangors_macros::Model;
#[derive(Model, Debug, Clone)]
#[djangors(app = "myapp", table_name = "contact")]
pub struct Contact {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(max_length = 100)]
    pub name: String,
    pub email: String,
}

async fn handle_submission(data: &std::collections::HashMap<String, String>) {
    match Contact::validate_form(data) {
        Ok(cleaned) => {
            let contact = Contact::from_cleaned_form(cleaned);
            // contact.save(&db).await
        }
        Err(errors) => {
            // errors.fields is a per-field HashMap<String, FieldError>, same shape
            // `djangors-forms`' own Form<T> extractor already uses
        }
    }
}
```

Auto/primary-key fields and `FileField`-kind fields are excluded from the generated form (the
same way `save()`'s own `INSERT` already skips auto fields). `apply_cleaned_form` applies a
validated submission onto an *existing* instance for the update path, leaving the primary key
untouched. HTML widget rendering is not part of this: pair it with `djangors-views`' generic
`CreateView`/`UpdateView` (see below) or build your own template.

---

## 9. REST Framework APIs

Django relies on Django REST Framework (DRF) `ModelSerializer` and `ViewSet`. Djangors provides `djangors-rest` with `serialize`, `deserialize`, and `ViewSet<M>`.

### Django REST Framework

```python
from rest_framework import serializers, viewsets
from .models import Article

class ArticleSerializer(serializers.ModelSerializer):
    class Meta:
        model = Article
        fields = '__all__'

class ArticleViewSet(viewsets.ModelViewSet):
    queryset = Article.objects.all()
    serializer_class = ArticleSerializer
```

### Djangors (`djangors-rest`)

```rust,compile
# #[derive(djangors_macros::Model, Debug, Clone, serde::Serialize)]
# #[djangors(app = "library", table_name = "library_article")]
# pub struct Article { #[djangors(primary_key, auto)] pub id: i64 }
# fn main() {
# let article_instance = Article { id: 1 };
use djangors_rest::{serialize, serialize_to_vec, viewset_routes};
use djangors_core::{Response, Router, StatusCode};

// Low-level model serialization to serde_json::Value or directly to UTF-8 JSON bytes
let json_val = serialize::<Article>(&article_instance);
let response = Response::json_bytes(StatusCode::OK, serialize_to_vec(&article_instance));

// High-level ViewSet registration. `ViewSet<M>` has no instance to construct;
// its CRUD handlers are mounted directly as a free function, IsAuthenticated by default.
let router = viewset_routes::<Article>(Router::new(), "/articles");
# let _ = (json_val, response, router);
# }
```

---

## 10. Conceptual Architecture Comparison

| Architectural Aspect | Django (Python) | Djangors (Rust) |
| :--- | :--- | :--- |
| **Execution Model** | Dynamic, interpreted (CPython WSGI/ASGI) | Compiled native binary (Tokio async loop) |
| **Type Checking** | Runtime duck typing, optional type hints | Strict compile-time static type system |
| **Concurrency** | Thread pool (WSGI) or async event loop (ASGI) | Native async task channels and multithreaded Tokio runtime |
| **Deployment Model** | Python interpreter + virtualenv + Gunicorn/Uvicorn + Nginx static serving | Compiled application binary plus configured static/file storage and dependencies |
| **Development Reloading** | Instant Python module re-import in dev server | File watcher trigger followed by `cargo` incremental binary re-compilation |
| **Memory Management** | Automatic garbage collection (ref counting + GC) | Ownership, borrowing, and RAII without garbage collection overhead |

---

## 11. The Django app pattern in Djangors

Django makes it easy to put a model, form, serializer, view, and URL in one
app. Djangors can do the same, but a real feature should make its boundaries
explicit as it grows:

```text
src/apps/library/
├── mod.rs              # public module surface and app wiring
├── models.rs           # #[derive(Model)] persistence records
├── contracts.rs        # request/path/query/response DTOs
├── errors.rs           # domain failures and HTTP translation
├── repositories.rs     # QuerySet and database access only
├── services.rs         # business rules, workflows, transactions
├── permissions.rs      # role and object/tenant access rules
├── serializers.rs      # REST representation and field visibility
├── views.rs            # Request -> Response adapters
├── urls.rs             # app-local Router
└── admin.rs            # admin registration, when needed

tests/apps/library/
├── models.rs
├── services.rs
├── permissions.rs
└── api.rs
```

The dependency direction is:

```text
urls -> views -> permissions/contracts -> services -> repositories -> ORM/DB
```

| Django convention | Djangors production boundary |
| --- | --- |
| `models.py` | `models.rs` |
| `forms.py` / serializer input | `contracts.rs`, `Form<T>`, or REST deserializer |
| `QuerySet` calls in a view | `repositories.rs` |
| model/service/business rules | `services.rs` |
| `Http404`, `PermissionDenied`, API exception | `errors.rs` -> `DjangorsError` |
| `views.py` | `views.rs` |
| `urls.py` | `urls.rs` |
| `admin.py` | `admin.rs` |
| Celery task | `#[task]` handler |
| `TestCase` / API client | `tests/apps/<app>/` and `djangors-test` |

The important translation is not “Django view becomes a Rust function.” It is
“Django’s implicit application conventions become explicit Rust modules.” For
a tiny endpoint, combining files is fine. Split the layers before a view owns
SQL, authorization, validation, error formatting, and task enqueueing together.

## 12. ORM translation: QuerySets, managers, and transactions

The Djangors ORM resembles Django’s QuerySet API, but its operations are typed
and asynchronous.

### Django

```python
books = (
    Book.objects
    .filter(is_published=True)
    .order_by("-created_at")[:10]
)
book = Book.objects.get(pk=book_id)
```

### Djangors

```rust,illustrative
use djangors_orm::{q, Model};

let books = Book::objects()
    .filter(q!(is_published = true))?
    .order_by("-created_at")?
    .limit(10)
    .all(db)
    .await?;

let book = Book::objects()
    .filter(q!(id = book_id))?
    .get(db)
    .await?;
```

The usual Rust differences are:

- import the `Model` trait before calling `Book::objects()`;
- handle query construction and database errors with `Result`;
- await database operations explicitly;
- use `q!()` for field expressions instead of assembling SQL strings;
- map `OrmError::NotFound` to a domain error at the service boundary;
- keep QuerySet construction in a repository once a feature has business logic.

For multi-step writes, use the database transaction APIs. A transaction is a
database consistency boundary; it is not a replacement for a service. The
service decides the workflow and the repository performs its queries.

## 13. Contracts, serializers, and public API shape

In Django, a `ModelSerializer` or form often becomes the public contract. In
Rust, make the contract explicit rather than deserializing into a model that
contains server-owned fields:

```rust,illustrative
#[derive(Debug, serde::Deserialize)]
pub struct CreateBook {
    pub title: String,
}

#[derive(Debug, serde::Serialize)]
pub struct BookResponse {
    pub id: i64,
    pub title: String,
    pub published: bool,
}
```

For CRUD endpoints, `ModelSerializer` and `FieldSet` provide DRF-like field
control:

```rust,illustrative
use djangors_rest::{FieldSet, ModelSerializer};

let serializer = ModelSerializer::<Book>::new(
    FieldSet::all()
        .read_only(&["id", "created_at"])
        .excluding(&["internal_notes"]),
);
```

Use `read_only` for primary keys, ownership, timestamps, computed fields, and
workflow status. Use `write_only` for passwords and tokens. Use a custom
`Serializer<M>` when the API shape differs from the database record. A
serializer validates representation-level input; a service still owns
cross-record validation and state transitions.

## 14. Errors and error rendering

Django’s `Http404`, `PermissionDenied`, DRF exception handler, and custom
exception classes become an app error enum plus a conversion to
`DjangorsError`:

```rust,illustrative
#[derive(Debug)]
pub enum BookError {
    Unauthenticated,
    Forbidden,
    NotFound,
    InvalidTitle,
    Persistence(String),
}

impl From<BookError> for djangors_core::DjangorsError {
    fn from(error: BookError) -> Self {
        use hyper::StatusCode;
        match error {
            BookError::Unauthenticated => djangors_core::DjangorsError::api(
                StatusCode::UNAUTHORIZED,
                "not_authenticated",
                "Authentication credentials were not provided.",
            ),
            BookError::Forbidden => djangors_core::DjangorsError::api(
                StatusCode::FORBIDDEN,
                "permission_denied",
                "You do not have permission to perform this action.",
            ),
            BookError::NotFound => djangors_core::DjangorsError::api(
                StatusCode::NOT_FOUND,
                "book_not_found",
                "Book was not found.",
            ),
            BookError::InvalidTitle => djangors_core::DjangorsError::api(
                StatusCode::BAD_REQUEST,
                "invalid_title",
                "Title is required.",
            ),
            BookError::Persistence(message) => {
                tracing::error!(error = %message, "book persistence failure");
                djangors_core::DjangorsError::Internal("Book operation failed.".into())
            }
        }
    }
}
```

Views return `Result<Response, DjangorsError>` and let the router render the
error. API errors default to JSON; browser errors can render HTML in debug or
production mode according to the request. Register a custom
`djangors_core::error::ErrorRenderer` in router state when every endpoint must
use one envelope:

```json
{
  "error": {
    "code": "book_not_found",
    "message": "Book was not found.",
    "details": null
  }
}
```

Never expose SQL, connection strings, stack traces, provider responses, or
Rust `Debug` variant names in production responses. Log diagnostic details
with `tracing`; return a safe client message. Attach field-level `details` for
validation errors.

## 15. Authentication, permissions, and tenant scope

`request.user` in Django becomes an authenticated user resolved from a Djangors
session, token, or JWT. `djangors_rest::current_user(&req)` supports the
configured mechanisms; `IsAuthenticated` is the standard route guard.

Keep these three questions separate:

```text
authentication: who is the caller?
permission:     may the caller perform this action?
scope:          which rows belong to the caller/tenant?
```

Use `IsStaff`, `IsSuperuser`, and composed permissions for common policies. For
tenant-owned models, implement `Scoped` and mount with
`scoped_viewset_routes` or `scoped_viewset_routes_with_config`. Scoping limits
rows; it does not automatically decide which role may edit them.

For a handwritten view, use this order:

1. Resolve the authenticated principal.
2. Resolve the tenant from trusted session/membership data.
3. Check the role or object permission.
4. Query by tenant and record ID together.
5. Validate every foreign key against that same tenant before writing.

Do not trust a tenant ID from a request body, and do not load an unscoped row
before checking its tenant.

## 16. Background work: Celery and management commands

Django’s Celery task becomes a serializable Djangors task:

```rust,illustrative
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SendReportArgs {
    pub report_id: i64,
}

#[djangors_tasks::task]
pub async fn send_report(
    args: SendReportArgs,
) -> Result<(), djangors_tasks::TaskError> {
    let db = crate::runtime::db();
    // Reload by ID, perform idempotent work, and persist the result.
    let _ = (db, args);
    Ok(())
}
```

Enqueue after the database state needed by the task is committed:

```rust,illustrative
djangors_tasks::enqueue(
    db,
    "send_report",
    &SendReportArgs { report_id },
).await?;
```

Task handlers do not receive `Request`, sessions, or a database reference.
Pass small IDs and immutable values; reload current state in the handler. The
database-backed worker records pending/running/completed/failed states, retries
failures up to `max_attempts`, and isolates panics. Make tasks idempotent and
treat an already-completed job as success.

Use `dj runworker` for the worker process and `register_recurring` for
five-field cron jobs. Create task tables and register handlers before the
worker starts. A short in-process `tokio::spawn` is fine for work that may be
lost on process restart; it is not a durable queue.

## 17. Testing translation

| Django test | Djangors equivalent |
| --- | --- |
| `TestCase` | unit/service tests plus `TestDatabase` |
| `Client` | `djangors_test::TestClient` or a real Hyper socket |
| `RequestFactory` | construct `Request` directly |
| `assertEqual(response.status_code, 404)` | `response.assert_status(StatusCode::NOT_FOUND)` |
| `override_settings` | construct typed settings/state for the test |
| `TransactionTestCase` | explicit database/transaction integration test |
| Celery eager task test | call the service directly, then test queue execution separately |

```rust,illustrative
#[tokio::test]
async fn missing_book_is_not_found() {
    let response = djangors_test::TestClient::new(crate::urls::urls())
        .get("/api/v1/books/999")
        .send()
        .await;
    response.assert_status(hyper::StatusCode::NOT_FOUND);
}
```

Organize production tests outside `src/`:

```text
tests/apps/library/
├── models.rs       # metadata and constraints
├── services.rs     # business rules without HTTP
├── permissions.rs  # roles and tenant isolation
├── api.rs          # real status/body/header contracts
└── tasks.rs        # serialization, retries, idempotency
```

Run SQLite for fast feedback, but also test PostgreSQL-specific behavior such
as locking, extensions, and SQL expressions against PostgreSQL.

## 18. Startup, settings, and middleware

`settings.py` is split into framework settings, typed application settings,
and explicit startup wiring:

```rust,illustrative
let (framework_settings, warnings) = djangors_core::DjangorsSettings::load()?;
for warning in warnings {
    eprintln!("settings warning: {warning}");
}

let db = djangors_db::Database::connect(
    &djangors_db::DatabaseConfig::new(database_url),
).await?;

let router = crate::urls::urls()
    .with_state(db.clone())
    .with_state(app_settings);
```

A production startup sequence is:

1. Load framework and application settings.
2. Initialize development or production logging.
3. Connect the database and run migrations.
4. Create task tables and install runtime handles.
5. Build Redis, cache, mail, and provider clients.
6. Register recurring jobs and start the worker.
7. Attach typed state and an optional `ErrorRenderer`.
8. Wrap `RouterService` with sessions, tenant resolution, security headers, CSRF, and logging layers.
9. Serve with graceful shutdown.

Unlike Django middleware strings in `MIDDLEWARE`, Djangors uses Tower’s
`ServiceBuilder`:

```rust,illustrative
let router_service = djangors_core::router::RouterService::new(router, debug);
let service = tower::ServiceBuilder::new()
    .layer(djangors_sessions::SessionLayer::new(session_store))
    .layer(djangors_core::middleware::security_headers_layer())
    .layer(djangors_core::middleware::csrf_layer())
    .service(router_service);
```

Use `/healthz` as a process liveness probe and `/readyz` as a dependency
readiness probe. A readiness failure should return `503` and a safe generic
message while logging the real database/cache error.

## 19. Migrations and schema ownership

`makemigrations` can inspect registered model metadata, but production schema
changes still need review. Keep migrations owned by the domain app, make
forward and rollback behavior explicit, and test foreign keys, unique indexes,
check constraints, and tenant constraints.

```bash
dj makemigrations
dj sqlmigrate library 0001
dj migrate --plan
dj migrate
dj showmigrations
```

Use `dj migrate --rollback` deliberately and never use `--fake` unless the
database already has exactly the schema represented by the migration. Do not
hide schema changes in a task or an application startup side effect.

## 20. What does not translate one-for-one

Some Django assumptions need a deliberate design choice:

- There is no automatic `reverse()` behavior unless you name routes with
  `.name("route-name")` and call `Router::reverse`.
- Rust has no dynamic model field access equivalent to arbitrary Python
  attribute lookup; field names, types, and relationships are checked earlier.
- A model is a plain struct, not a place to put request context or global
  service dependencies.
- Djangors tasks are database-backed and durable; a raw Tokio task is not.
- HTML rendering and API serialization are separate choices; do not return
  database rows directly just because they implement `Serialize`.
- Djangors provides both SSE (`Router::sse` / `Response::sse`) and RFC 6455
  WebSockets (`Router::ws` / `djangors_core::ws::WebSocket`, plus pub/sub
  broadcast channels via `djangors_core::Groups`).
- SQLite is a useful test backend (and production backend for single-node apps)
  but does not prove PostgreSQL locking, extensions, or dialect-specific behavior.

The migration is complete when every endpoint has an explicit route, contract,
permission rule, tenant rule where applicable, service workflow, repository
query, error mapping, response shape, migration, and tests—not merely when the
Rust code compiles.
