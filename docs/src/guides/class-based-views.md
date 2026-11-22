# Class-Based Views (`djangors-views`)

`djangors-views` provides server-rendered generic class-based views — Django's
`django.views.generic` equivalent in Djangors. It ships five generic views
(`ListView`, `DetailView`, `CreateView`, `UpdateView`, `DeleteView`) that turn a
`#[derive(Model)]` type into CRUD pages rendered through the
[template engine](../guides/templates.md).

## When to reach for it

Reach for `djangors-views` when you are building a **server-rendered HTML
application** — pages that return `text/html` rendered from templates, where
forms submit via `application/x-www-form-urlencoded` and the browser follows a
redirect on success.

It is *not* the right tool for JSON APIs. For a JSON/`fetch()`-based API over
the same model use `djangors-rest`'s `ViewSet` (see the
[REST Framework guide](../guides/rest.md)), which emits `list`/`create`/
`retrieve`/`update`/`destroy` routes and serialized JSON bodies instead of
rendered HTML.

## The five views

| View | Signature (generic bound) | Renders / redirects |
| --- | --- | --- |
| `ListView<M>` | `M: Model + FromRow` | Renders `object_list` |
| `DetailView<M>` | `M: Model + FromRow` | Renders `object` (looked up by `pk`) |
| `CreateView<M>` | `M: Model + FromRow + ModelForm` | GET renders a blank form; POST inserts + redirects |
| `UpdateView<M>` | `M: Model + FromRow + ModelForm` | GET pre-fills a form; POST updates by `pk` + redirects |
| `DeleteView<M>` | `M: Model + FromRow` | GET renders a confirm page; POST deletes by `pk` + redirects |

Every view is an `async fn` that takes `(req: Request, params: PathParams,
config: &ViewSetConfig)` and returns `Result<Response, DjangorsError>`.

## Configuration (`ViewSetConfig`)

Each view needs a [`ViewSetConfig`](file:///root/dev/Rango/crates/djangors-views/src/lib.rs)
that tells it which `TemplateEngine` to render with, which template to use, and
where to redirect after a successful write:

```rust,compile
# use std::sync::LazyLock;
# use djangors_template::TemplateEngine;
# use djangors_views::ViewSetConfig;
#
# fn main() {
# static TEMPLATES: LazyLock<TemplateEngine> = LazyLock::new(|| {
#     TemplateEngine::new(vec!["templates".into()]).expect("load templates")
# });
#
// The engine is a `&'a TemplateEngine` inside the config, so it must outlive
// every handler. A `static` (via `LazyLock`) is the usual way to satisfy that.
let config = ViewSetConfig {
    engine: &*TEMPLATES,
    template_name: "polls/question_list.html",
    success_url: "/polls/",
};
# let _ = &config;
# }
```

## Registering all five views on a `Router`

`Router` handlers are `Fn(Request, PathParams) -> Future`; the view functions
take the config as an extra argument, so wrap each one in a closure that builds
(or references) its config. Because the config borrows the engine, building the
config inside the closure from a `'static` engine keeps the closures `'static`:

```rust,compile
use std::sync::LazyLock;
use djangors_core::Router;
use djangors_template::TemplateEngine;
use djangors_views::{CreateView, DeleteView, DetailView, ListView, UpdateView, ViewSetConfig};
use polls::models::{Choice, Question};

static TEMPLATES: LazyLock<TemplateEngine> = LazyLock::new(|| {
    TemplateEngine::new(vec!["templates".into()]).expect("load templates")
});

fn question_routes() -> Router {
    Router::new()
        .get("/polls/", |req, params| async move {
            let config = ViewSetConfig {
                engine: &*TEMPLATES,
                template_name: "polls/question_list.html",
                success_url: "/polls/",
            };
            ListView::<Question>::list(req, params, &config).await
        })
        .post("/polls/new/", |req, params| async move {
            let config = ViewSetConfig {
                engine: &*TEMPLATES,
                template_name: "polls/question_form.html",
                success_url: "/polls/",
            };
            CreateView::<Question>::create(req, params, &config).await
        })
        .get("/polls/{pk}/", |req, params| async move {
            let config = ViewSetConfig {
                engine: &*TEMPLATES,
                template_name: "polls/question_detail.html",
                success_url: "/polls/",
            };
            DetailView::<Question>::detail(req, params, &config).await
        })
        .post("/polls/{pk}/edit/", |req, params| async move {
            let config = ViewSetConfig {
                engine: &*TEMPLATES,
                template_name: "polls/question_form.html",
                success_url: "/polls/",
            };
            UpdateView::<Question>::update(req, params, &config).await
        })
        .post("/polls/{pk}/delete/", |req, params| async move {
            let config = ViewSetConfig {
                engine: &*TEMPLATES,
                template_name: "polls/question_confirm_delete.html",
                success_url: "/polls/",
            };
            DeleteView::<Question>::delete(req, params, &config).await
        })
        .post("/choices/new/", |req, params| async move {
            let config = ViewSetConfig {
                engine: &*TEMPLATES,
                template_name: "polls/choice_form.html",
                success_url: "/polls/",
            };
            CreateView::<Choice>::create(req, params, &config).await
        })
}
```

> [!IMPORTANT]
> The `{pk}` path param is read via `PathParams::get_as::<i64>("pk")`. The
> route must expose a segment named `pk`, so name it `{pk}` (or `{pk:i64}`) in
> the path pattern — not `{id}` / `{question_id}`.

## Template context variables

Each view renders the configured template with a fixed set of context variables:

| View | `object_list` | `object` | `fields` | `form` | `errors` |
| --- | :---: | :---: | :---: | :---: | :---: |
| `ListView` | all rows | — | — | — | — |
| `DetailView` | — | one row | — | — | — |
| `CreateView` (GET) | — | — | field names | — | `{}` empty |
| `CreateView` (POST error) | — | — | field names | submitted data | per-field + `__all__` |
| `UpdateView` (GET) | — | one row | field names | — | `{}` empty |
| `UpdateView` (POST error) | — | one row | — | submitted data | per-field + `__all__` |
| `DeleteView` (GET) | — | one row | — | — | — |

- **`object_list`** — an array of row objects, each keyed by field name (e.g.
  `{{ o.question_text }}`).
- **`object`** — a single row object keyed by field name (e.g.
  `{{ object.question_text }}`).
- **`fields`** — the model's field names (from `Model::field_names()`), useful
  for rendering an empty form's inputs.
- **`form`** — the submitted `HashMap<String, String>` on a validation error,
  so you can re-populate the inputs (`value="{{ form['question_text'] }}"`).
- **`errors`** — an object mapping each invalid field name to its error message,
  plus a `__all__` key for non-field errors. On the initial GET it is an empty
  object `{}`.

## The HTML form contract (`CreateView` / `UpdateView`)

`CreateView` and `UpdateView` expect an `application/x-www-form-urlencoded`
`POST` body whose field names match the model's field names. The model must
implement `djangors_orm::ModelForm` — which `#[derive(Model)]` provides by
generating `validate_form()`, `from_cleaned_form()`, and `apply_cleaned_form()`.

- **`CreateView` (POST)** — submitted data is parsed with the
  [`Form` extractor](../guides/forms.md), then `M::validate_form(&data)` runs.
  On success the cleaned values build a new instance via `from_cleaned_form()`,
  the row is inserted, and the response redirects to `success_url`. On failure
  the form template is re-rendered with `fields`, `form`, and `errors`.
- **`UpdateView` (POST)** — the row for `pk` is fetched, then
  `M::validate_form(&data)` runs. On success `apply_cleaned_form()` mutates the
  fetched instance, all non-primary-key fields are written back, and the
  response redirects to `success_url`. On failure the form template is
  re-rendered with `object`, `form`, and `errors`.

A minimal create form template for `Question`:

```jinja
<form method="post" action="/polls/new/">
  <label>Question text
    <input name="question_text" value="{{ form['question_text'] }}">
  </label>
  <label>Published
    <input name="pub_date" type="datetime-local" value="{{ form['pub_date'] }}">
  </label>
  {% if errors['question_text'] %}<span>{{ errors['question_text'] }}</span>{% endif %}
  <button type="submit">Create</button>
</form>
```

## Prerequisites

- The model must be a `#[derive(Model)]` type.
  - `ListView` / `DetailView` / `DeleteView` need `M: Model + FromRow`
    (both provided by `#[derive(Model)]`).
  - `CreateView` / `UpdateView` additionally need `M: ModelForm` — also
    generated by `#[derive(Model)]`.
- A `djangors_orm::djangors_db::Database` must be in the request state, because
  every view calls `req.require_state::<Database>()` to fetch rows. Attach it
  with `Router::with_state(db)` (or `Request::with_state`).
- A `TemplateEngine` must be passed via `config.engine`. The views only read the
  engine from the config — the engine is **not** pulled from request state, so
  you are free to keep it anywhere (a `static`, app state, etc.) as long as it
  outlives the handlers.
- The configured `template_name` must exist in one of the engine's search
  directories, or rendering fails with an internal error.

> [!NOTE]
> The `Database` comes from request state, but the `TemplateEngine` comes
> exclusively from the config. These are two separate mechanisms — don't assume
> the engine is discoverable from the request.

## Limitations

- **No authentication or permissions built in.** Views return `Response`
  directly and perform no login/permission checks. Wrap them in your own
  middleware, or check the current user inside a wrapper handler, using
  `djangors-auth` (see the [Authentication guide](../guides/auth.md)).
- **No pagination.** `ListView` fetches every row with `QuerySet::all()` and
  renders them all. There is no page-size or pagination control.
- **`ListView` / `DetailView` are render-only.** They do not branch on the HTTP
  method — they always render. Only `CreateView` / `UpdateView` / `DeleteView`
  distinguish GET from POST (anything that is not GET is treated as POST).
- **URL-encoded forms only.** `CreateView` / `UpdateView` parse the body with
  `Form::<HashMap<String, String>>::from_request`, i.e. multipart or file uploads
  are not handled.
- **Integer primary keys.** Records are looked up and deleted by an `i64` `pk`
  (`params.get_as::<i64>("pk")`); a non-integer `pk` is a `BadRequest`.
- **The `pk` name is fixed.** Detail/update/delete look up the `pk` path
  parameter by name, so your route must use `{pk}`.

---

## Base Generic Views (`TemplateView`, `RedirectView`, `FormView`)

Djangors 1.0 adds Django's three non-model base views:

| View | Purpose | Key Builder Methods |
| --- | --- | --- |
| `TemplateView` | Renders a static or context-enriched template | `TemplateView::new(engine, template_name).with_context(json!(...))` |
| `RedirectView` | Returns `302 Found` or `301 Moved Permanently` | `RedirectView::new(url).permanent(true).query_string(true)` |
| `FormView` | `GET` renders form; `POST` validates via closure and redirects on success | `FormView::new(engine, template_name, success_url).with_context(...)` |

```rust,illustrative
use djangors_core::Router;
use djangors_views::{FormView, RedirectView, TemplateView};
use std::collections::HashMap;

fn base_view_routes() -> Router {
    Router::new()
        .get("/about/", |req, params| async move {
            TemplateView::new(&*TEMPLATES, "about.html")
                .with_context(serde_json::json!({ "version": "1.0" }))
                .dispatch(req, params)
                .await
        })
        .get("/old-home/", |req, params| async move {
            RedirectView::new("/")
                .permanent(true)
                .query_string(true)
                .dispatch(req, params)
                .await
        })
        .post("/contact/", |req, params| async move {
            FormView::new(&*TEMPLATES, "contact.html", "/contact/thanks/")
                .dispatch(req, params, |data| {
                    if data.get("message").map(|s| s.trim().is_empty()).unwrap_or(true) {
                        let mut errs = HashMap::new();
                        errs.insert("message".into(), "Message is required.".into());
                        Err(errs)
                    } else {
                        Ok(())
                    }
                })
                .await
        })
}
```

## Date-Based Archive Views

Djangors 1.0 provides four date-based archive views configured with **`DateArchiveConfig`**:

| View | Path Parameters | Template Context |
| --- | --- | --- |
| `ArchiveIndexView<M>::list` | *(none)* | `object_list` (newest first), `date_field` |
| `YearArchiveView<M>::list` | `{year}` (`i32`) | `object_list`, `year`, `date_field` |
| `MonthArchiveView<M>::list` | `{year}`, `{month}` (`1..=12`) | `object_list`, `year`, `month`, `date_field` |
| `DayArchiveView<M>::list` | `{year}`, `{month}`, `{day}` | `object_list`, `year`, `month`, `day`, `date_field` |

Set `.allow_empty(false)` on `DateArchiveConfig` to return `404 Not Found` when a date range has no matching rows (default `true`).

```rust,illustrative
use djangors_core::Router;
use djangors_views::{
    ArchiveIndexView, DateArchiveConfig, DayArchiveView, MonthArchiveView, YearArchiveView,
};

fn archive_routes() -> Router {
    Router::new()
        .get("/archive/", |req, params| async move {
            let cfg = DateArchiveConfig::new(&*TEMPLATES, "polls/archive.html", "pub_date");
            ArchiveIndexView::<Question>::list(req, params, &cfg).await
        })
        .get("/archive/{year:i64}/", |req, params| async move {
            let cfg = DateArchiveConfig::new(&*TEMPLATES, "polls/archive_year.html", "pub_date")
                .allow_empty(false);
            YearArchiveView::<Question>::list(req, params, &cfg).await
        })
        .get("/archive/{year:i64}/{month:i64}/", |req, params| async move {
            let cfg = DateArchiveConfig::new(&*TEMPLATES, "polls/archive_month.html", "pub_date");
            MonthArchiveView::<Question>::list(req, params, &cfg).await
        })
        .get("/archive/{year:i64}/{month:i64}/{day:i64}/", |req, params| async move {
            let cfg = DateArchiveConfig::new(&*TEMPLATES, "polls/archive_day.html", "pub_date");
            DayArchiveView::<Question>::list(req, params, &cfg).await
        })
}
```