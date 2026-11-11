# REST Framework

`djangors-rest` is the API layer: ViewSets that turn a model into CRUD routes,
serializers that shape request and response bodies, and pluggable pagination,
filtering, permissions, and throttling. It is closely modelled on Django REST
Framework.

## Trait imports: the four you always need

Every method below is a *trait method*; Rust does not bring it into scope
unless the trait is imported. Getting "method not found" in your first API
handler is almost always one of these four missing `use` lines:

| Method | Missing import |
| --- | --- |
| `Question::objects()` | `use djangors_orm::Model;` |
| `serializer.to_representation(..)` | `use djangors_rest::Serializer;` |
| `perm.has_permission(..)` | `use djangors_rest::Permission;` |
| `Json::<T>::from_request(..)` | `use djangors_core::extract::FromRequest;` |
| `current_user(&req)` | `use djangors_rest::current_user;` |

`djangors-orm` re-exports `Model` in its own prelude, but the other three live
in their crates' roots only because `djangors-rest` re-exports
`pub use permissions::*` and `pub use serializers::*` — the `use` statement is
still required.

## A minimal ViewSet

`viewset_routes` mounts the five standard actions for a model. Routes require an
authenticated user by default; opt out explicitly with
`viewset_routes_with_permission` and `AllowAny`.

| Method | Path | Action |
| --- | --- | --- |
| `GET` | `/` | `list` |
| `POST` | `/` | `create` |
| `GET` | `/{pk}` | `retrieve` |
| `PUT` / `PATCH` | `/{pk}` | `update` |
| `DELETE` | `/{pk}` | `destroy` |

`PATCH` is a genuine partial update: fields the body omits are left untouched
rather than reset to their defaults.

## Choosing a route-mounting function

`viewset_routes` is only the default. Five mounting functions exist, each a
shorthand that fills in defaults and delegates to the most complete one
(`viewset_routes_with_options`). Pick the one that matches how much you need to
customise:

| Function | Custom permission | Custom config | Custom serializer / pagination / throttle |
| --- | --- | --- | --- |
| `viewset_routes::<M>(router, base)` | No (`IsAuthenticated`) | No | No |
| `viewset_routes_with_permission::<M,P>(router, base, perm)` | Yes | No | No |
| `viewset_routes_with_config::<M>(router, base, config)` | No | Yes | No |
| `viewset_routes_with_config_and_permission::<M,P>(router, base, config, perm)` | Yes | Yes | No |
| `viewset_routes_with_options::<M,P>(router, base, options, perm)` | Yes | Yes (via options) | Yes |
| `scoped_viewset_routes::<M>(router, base)` | No (`IsAuthenticated`) | No | No — mandates `M: Scoped` |
| `scoped_viewset_routes_with_config::<M>(router, base, config)` | No (`IsAuthenticated`) | Yes | No — mandates `M: Scoped` |

```rust,illustrative
use djangors_rest::{
    AllowAny, FieldFilter, IsAuthenticated, OrderingFilter, SearchFilter, ViewSetConfig,
    ViewSetOptions, viewset_routes_with_config, viewset_routes_with_config_and_permission,
    viewset_routes_with_options, viewset_routes_with_permission,
};
use djangors_core::Router;
use polls::models::Question;

// Public read-only blog index: let anyone list, but only allow the fields below.
let public = viewset_routes_with_permission::<Question, _>(
    Router::new(),
    "/questions/public",
    AllowAny,
);

// Same default auth, but with a custom config (filtering + ordering allowlists).
let config = ViewSetConfig {
    filterable_fields: &["pub_date"],
    orderable_fields: &["pub_date"],
    ..Default::default()
};
let filtered = viewset_routes_with_config::<Question>(Router::new(), "/questions/filtered", config);

// Both.
let both = viewset_routes_with_config_and_permission::<Question, _>(
    Router::new(),
    "/questions/both",
    config,
    IsAuthenticated,
);

// Full control: a custom serializer, pagination strategy, filter backends, and a throttle.
let options = ViewSetOptions::<Question>::new(ViewSetConfig::default())
    .with_filter_backend(FieldFilter::new(&["id", "pub_date"]))
    .with_filter_backend(OrderingFilter::new(&["pub_date"]));
let full = viewset_routes_with_options::<Question, _>(
    Router::new(),
    "/questions/full",
    options,
    IsAuthenticated,
);
# let _ = (public, filtered, both, full);
```

> [!TIP]
> `_with_config` passes the config to the **list** handler only. `_with_options`
> is different: the options reach **every** handler, so a serializer's
> read/write field split actually applies to `create` and `update` as well as to
> `list` and `retrieve`. If you set read-only fields, use `_with_options`, not
> `_with_config`.

## Calling ViewSet actions directly from your own handler

Mounting with `viewset_routes*` is not the only way to use a ViewSet. Every
action is a plain `async fn` on `ViewSet::<M>` that you can call from your own
handler — useful when you want your own routing, middleware, or permission logic
around the standard CRUD while still reusing the framework's serializer and
pagination. The `_with_config` variants apply the filter/order allowlists; the
`_with_options` variants apply the full `ViewSetOptions` (serializer, pagination
strategy, filter backends, and throttle).

| Call | Applies |
| --- | --- |
| `ViewSet::<M>::list_with_config(req, params, &config)` | filter / order allowlists |
| `ViewSet::<M>::list_with_options(req, params, &options)` | serializer + pagination + backends + throttle |
| `ViewSet::<M>::retrieve_with_options(req, params, &options)` | serializer |
| `ViewSet::<M>::create_with_options(req, params, &options)` | serializer parse + validate |
| `ViewSet::<M>::update_with_options(req, params, &options)` | serializer parse + validate (`PATCH` = partial) |

> [!NOTE]
> These handlers do **not** check permissions — that is the job of the `guarded`
> wrapper `viewset_routes*` applies. Call your own permission check (or route
> through `IsAuthenticated`) before delegating, and ensure a `Database` is in
> the request state.

```rust,compile
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_rest::{FieldSet, ModelSerializer, ViewSet, ViewSetConfig, ViewSetOptions};
use polls::models::Question;

// Your own handler that reuses the framework's serializer + pagination logic.
pub async fn my_list(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let options = ViewSetOptions::<Question>::new(ViewSetConfig::default())
        .with_serializer(ModelSerializer::<Question>::new(FieldSet::all().read_only(&["id"])));
    ViewSet::<Question>::list_with_options(req, params, &options).await
}
```

## Mandatory row-level scoping (`Scoped` / `ScopedViewSet`)

A ViewSet that should only ever touch rows belonging to the current user, school,
or tenant is a `ScopedViewSet`. The model must implement `Scoped`, and — because
that is a hard `where M: Scoped` bound — a model that does **not** implement it
simply will not compile against `ScopedViewSet`. You cannot forget the scope.

```rust,compile
use djangors_core::{Request, error::DjangorsError};
use djangors_macros::Model;
use djangors_orm::QuerySet;
use djangors_rest::Scoped;

#[derive(Model, Debug, Clone)]
#[djangors(app = "notes", table_name = "notes_note")]
struct Note {
    #[djangors(primary_key, auto)]
    id: i64,
    #[djangors(foreign_key(on_delete = "cascade"))]
    author: djangors_orm::ForeignKey<djangors_auth::User>,
    #[djangors(max_length = 500)]
    body: String,
}

impl Scoped for Note {
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError> {
        // Resolve the current user (see `current_user` below), then filter the
        // queryset to rows they own. Every query through this endpoint applies
        // this filter — there is no un-scoped escape hatch.
        let user_id = 1i64;
        qs.filter(djangors_orm::q!(author = user_id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))
    }
}

// Mount: all five CRUD actions, each one scoped to the current user.
# fn mount(router: djangors_core::Router) -> djangors_core::Router {
djangors_rest::scoped_viewset_routes::<Note>(router, "/notes")
# }
```

`scoped_viewset_routes` takes no `ViewSetConfig`. If you need filtering or
ordering on a scoped endpoint, reach for `scoped_viewset_routes_with_config`
first — it mounts the same five CRUD routes with a custom config on `list`,
and wraps every one of them in `IsAuthenticated` for you (see the
[Multi-tenancy guide](multi-tenancy.md) for the `tenant_scope` helper).

> [!WARNING]
> `ScopedViewSet::<M>`'s associated functions (`list_with_config`, `retrieve`,
> `create`, `update`, `destroy`) check **only** `Scoped::scope` — the same "no
> built-in permission check" rule from the note above applies here too, and
> the consequence is sharper: `scope` decides *which rows* are visible
> (typically "this tenant"), not *who's allowed to write*. If you mount these
> functions yourself instead of going through `scoped_viewset_routes*`, every
> authenticated member of that scope gets full read/write access regardless of
> role — there is no framework-enforced distinction between "can view my own
> school's records" and "can edit anyone's." Prefer the `scoped_viewset_routes*`
> helpers; if you must hand-mount, add your own `IsAuthenticated` (or stricter)
> check before delegating, and encode any role restriction directly in
> `Scoped::scope` or a wrapping handler.

## Resolving the current user (`current_user`)

`current_user(&req)` resolves the authenticated user from the same sources
`IsAuthenticated` checks — session auth, then API token, then JWT when enabled —
returning `None` for an unauthenticated request. It is the one utility that
covers all three auth mechanisms in a single call, so use it instead of reaching
for a specific extractor.

```rust,compile
use djangors_core::{Request, error::DjangorsError};
use djangors_rest::current_user;

pub async fn me(req: Request) -> Result<String, DjangorsError> {
    match current_user(&req).await {
        Some(user) => Ok(format!("hello, {}", user.username)),
        None => Err(DjangorsError::Unauthorized("not logged in".into())),
    }
}
```

Note that `current_user` returns `Option<djangors_auth::User>` — it never
errors, so combine it with `IsAuthenticated` at the mount point when you need
the route to reject unauthenticated callers up front.

## Serializers

A serializer decides which fields are readable, which are writable, and what
counts as valid. `ModelSerializer` derives all of that from the model's
metadata; `FieldSet` narrows it.

```rust,compile
# use djangors_rest::{FieldSet, ModelSerializer};
# use polls::models::Question;
# fn main() {
// Expose everything except `pub_date`, and never accept `id` on a write.
let serializer = ModelSerializer::<Question>::new(
    FieldSet::all().excluding(&["pub_date"]).read_only(&["id"]),
);
# let _ = serializer;
# }
```

`FieldSet` builds on an implicit "everything" baseline and narrows it. The full
syntax:

```rust,compile
# use djangors_rest::FieldSet;
# fn main() {
// Only these two fields, and `id` is still never writable:
let f1 = FieldSet::only(&["id", "question_text"]);

// Everything-with-some-dropped, then some marked read-only / write-only:
let f2 = FieldSet::all()
    .excluding(&["questions"])      // hide a relation entirely
    .read_only(&["id", "pub_date"]) // render but never accept on write
    .write_only(&["votes"]);        // accept on write, never render
# let _ = (f1, f2);
# }
```

- **`read_only`** fields are rendered but rejected on write, rather than
  silently ignored.
- **`write_only`** fields (passwords, tokens) are accepted on write and never
  appear in a response.

### Validation

Field errors accumulate into `ValidationErrors` and render as a `422` whose
`details` carries the `{field: [messages]}` map, so a client sees every problem
at once instead of one per round trip.

```rust,compile
# use djangors_rest::ValidationErrors;
# fn main() {
let mut errors = ValidationErrors::new();
errors.add("question_text", "This field may not be blank.");
errors.add_non_field("Either a question or a choice is required.");
# let _ = errors;
# }
```

### `ValidationErrors`: the full API

Beyond the `add` / `add_non_field` shown above, the error set supports
everything you need to merge, inspect, and ship errors:

| Method | Behaviour |
| --- | --- |
| `add(field, msg)` | append a message to a named field |
| `add_non_field(msg)` | an object-level message (DRF's `non_field_errors`) |
| `merge(other)` | fold another set in, preserving every message per field |
| `is_empty()` | `true` when nothing has been recorded |
| `get(field)` | `Option<&[String]>` for one field |
| `contains_key(field)` | whether a field has any message |
| `field_names()` | iterator over the fields that have messages |
| `non_field_errors()` | `&[String]` of object-level messages |
| `into_result()` | `Ok(())` when empty, otherwise `Err(self)` |
| `to_json()` | the `{field: [msg]}` map (object-level under `non_field_errors`) |

`VALIDATION_ERROR_CODE` is the stable `code` the set renders into — branch on it
instead of parsing a message string.

```rust,compile
# use djangors_rest::{ValidationErrors, VALIDATION_ERROR_CODE};
# fn main() {
let mut a = ValidationErrors::new();
a.add("question_text", "must not be blank");
a.add("pub_date", "must be in the future");

let mut b = ValidationErrors::new();
b.add("question_text", "too long");
b.add_non_field("this question looks wrong");

a.merge(b); // both `question_text` messages survive, plus the non-field one

assert!(!a.is_empty());
assert_eq!(a.get("question_text").unwrap().len(), 2);
assert!(a.contains_key("pub_date"));
let fields: Vec<&str> = a.field_names().collect();
let non_field: &[String] = a.non_field_errors();
let json = a.to_json(); // {"question_text": [...], "pub_date": [...], "non_field_errors": [...]}
let _ = a.clone().into_result(); // Ok(()) when empty, Err(self) otherwise
assert_eq!(VALIDATION_ERROR_CODE, "validation_error");
# let _ = (fields, non_field, json);
# }
```

### Low-level `serialize`, `serialize_to_vec`, and `serialize_many_to_vec`

When you only need a model rendered to JSON without a full ViewSet — a custom
handler, a CSV/export endpoint, a debug route — call `djangors_rest::serialize`
for a `serde_json::Value`, or `serialize_to_vec` / `serialize_many_to_vec` to
stream directly into a `Vec<u8>` buffer (paired with `Response::json_bytes`)
using the compile-time `#[derive(Model)]` JSON writer without allocating
intermediate `serde_json::Value` maps:

```rust,compile
# use polls::models::Question;
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
use djangors_core::{Response, StatusCode};
use djangors_orm::Model;
use djangors_rest::{serialize, serialize_many_to_vec};

let questions = Question::objects().all(db).await?;
let json = serde_json::Value::Array(questions.iter().map(serialize).collect());
let response = Response::json_bytes(StatusCode::OK, serialize_many_to_vec(&questions));
# let _ = (json, response);
# Ok(())
# }
```

### Nested serializers

Because the serializer trait is synchronous it never issues queries itself.
Load the relation with `select_related`, then render it with
`NestedSerializer`, which embeds the related object in place of the raw foreign
key. When no related instance is supplied the field keeps its id, so a missing
join degrades to the flat representation rather than to `null`.

```rust,compile
# use djangors_rest::{ModelSerializer, NestedSerializer};
# use polls::models::{Choice, Question};
# fn main() {
let serializer = NestedSerializer::new(
    ModelSerializer::<Choice>::default(),
    "question",
    ModelSerializer::<Question>::default(),
);
# let _ = serializer;
# }
```

### Rendering preloaded relations (`render` / `render_many`)

`NestedSerializer` renders by hand rather than through a ViewSet: build it once,
then call `render(&self, instance, related: Option<&R>)` for a single row, or
`render_many(&self, rows: &[(M, Option<R>)])` for a collection — the exact shape
`select_related` returns. When the related row is `None`, the field keeps its raw
id, degrading to the flat representation rather than to `null`.

```rust,compile
# use polls::models::{Choice, Question};
# use djangors_rest::{FieldSet, ModelSerializer, NestedSerializer};
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
use djangors_orm::Model;

let serializer = NestedSerializer::new(
    ModelSerializer::<Choice>::default(),
    "question",
    ModelSerializer::<Question>::new(FieldSet::all().excluding(&["pub_date"])),
);

// One query loads both tables; each element is (Choice, Option<Question>).
let rows: Vec<(Choice, Option<Question>)> =
    Choice::objects().select_related::<Question, _>(db, "question").await?;

// Embed each question under the `question` key (or keep the raw id when the FK is dangling).
let json = serializer.render_many(&rows);
# let _ = json;
# Ok(())
# }
```

### Writing a custom `Serializer<M>`

`ModelSerializer` + `FieldSet` can only select, exclude, or redirect *whole model
fields* — it cannot compute, fuse, or rename a field. For that, implement the
`Serializer<M>` trait yourself. It is a small surface, fully overridable:

| Method | Purpose | Default |
| --- | --- | --- |
| `to_representation(&self, &M) -> serde_json::Value` | render one row for the response body | — (required) |
| `to_internal_value(&self, &Value, partial) -> Result<FieldValues, ValidationErrors>` | parse + validate a body into column values | — (required) |
| `validate(&self, &[(&str, Value)], &mut ValidationErrors)` | object-level rules, run after field parsing | no-op |
| `to_representation_nested(&self, &M, &RelatedObjects) -> Value` | embed preloaded relations in place of FK ids | merges the base representation |
| `to_representation_many(&self, &[M]) -> Vec<Value>` | render a collection | maps `to_representation` |
| `parse(&self, &Value, partial) -> Result<FieldValues, ValidationErrors>` | `to_internal_value` then `validate` | ViewSets call this |

ViewSets call `parse`, never `to_internal_value` directly, so a `validate`
override is never accidentally skipped. `NestedSerializer` renders through
`to_representation_nested`; the default implementation inlines each preloaded
relation into the base representation, which is why overriding
`to_representation` is enough for most custom shapes.

```rust,compile
use polls::models::{Choice, Question};
use djangors_rest::{FieldValues, Serializer, ValidationErrors};

/// Renames `question_text` to `display_name`, computed from the question's id
/// and text — impossible with `FieldSet`, which only selects whole fields.
struct QuestionSummary;

impl Serializer<Question> for QuestionSummary {
    fn to_representation(&self, question: &Question) -> serde_json::Value {
        serde_json::json!({
            "id": question.id,
            "display_name": format!("{}: {}", question.id, question.question_text),
        })
    }

    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<FieldValues, ValidationErrors> {
        // `display_name` is read-only; the write side stays boring by delegating
        // to the low-level column-name deserializers.
        let values = if partial {
            djangors_rest::deserialize_partial::<Question>(data)?
        } else {
            djangors_rest::deserialize::<Question>(data)?
        };
        Ok(values)
    }
}

/// Same idea for choices: `display_name` fuses `choice_text` and `votes`.
struct ChoiceSummary;

impl Serializer<Choice> for ChoiceSummary {
    fn to_representation(&self, choice: &Choice) -> serde_json::Value {
        serde_json::json!({
            "id": choice.id,
            "display_name": format!("{} ({} votes)", choice.choice_text, choice.votes),
        })
    }

    fn to_internal_value(
        &self,
        data: &serde_json::Value,
        partial: bool,
    ) -> Result<FieldValues, ValidationErrors> {
        let values = if partial {
            djangors_rest::deserialize_partial::<Choice>(data)?
        } else {
            djangors_rest::deserialize::<Choice>(data)?
        };
        Ok(values)
    }
}
```

Wire a custom serializer in with `ViewSetOptions::with_serializer` inside
`viewset_routes_with_options` (see the
[route-mounting table](#choosing-a-route-mounting-function)).

### Object-level validation (`Validator<T>` / `with_validator`)

Per-field rules live inside `to_internal_value`. Cross-field *business* rules —
"end must be after start", "at least one of X or Y" — attach to a
`ModelSerializer` with `with_validator`. `Validator<T>` is implemented for any
closure `Fn(&T, &mut ValidationErrors)`, and `ModelSerializer` runs validators on
the parsed `FieldValues` in registration order, after fields parse successfully.
Every registered validator runs even if an earlier one failed, so the client sees
the full set of problems at once.

```rust,compile
# use polls::models::Question;
# fn main() {
use djangors_rest::{FieldSet, FieldValues, ModelSerializer, ValidationErrors};

let serializer = ModelSerializer::<Question>::new(FieldSet::all()).with_validator(
    |values: &FieldValues, errors: &mut ValidationErrors| {
        // Object-level rule: a question must carry actual text, not just an id.
        let has_text = values.iter().any(|(name, _)| *name == "question_text");
        if !has_text {
            errors.add_non_field("a question needs question_text");
        }
    },
);
# let _ = serializer;
# }
```

`with_validator` returns `Self`, so you can chain several validators; each one
runs in order after the fields parse.

## Pagination

Three strategies ship, all behind the `Pagination` trait:

- **`PageNumberPagination`** — `?page=2`, the default.
- **`LimitOffsetPagination`** — `?limit=20&offset=40`.
- **`CursorPagination`** — opaque cursor, stable under concurrent inserts and
  issues no `COUNT`.

Page size is server-controlled by default. Clients may only override it with
`?page_size=` when `max_page_size` opts in, and the value is clamped to that
cap.

Each strategy is a plain struct with public fields, so you can construct one
directly or use `Default`. All three derive their page-size defaults from the
shared `REST_PER_PAGE` constant (100):

| Struct | Field | Default |
| --- | --- | --- |
| `PageNumberPagination` | `page_size: i64` | `100` |
| `PageNumberPagination` | `max_page_size: Option<i64>` | `None` (no client override) |
| `LimitOffsetPagination` | `default_limit: i64` | `100` |
| `LimitOffsetPagination` | `max_limit: i64` | `100` |
| `CursorPagination` | `page_size: i64` | `100` |
| `CursorPagination` | `max_page_size: Option<i64>` | `None` |

```rust,compile
# use djangors_rest::{LimitOffsetPagination, PageNumberPagination, REST_PER_PAGE};
# fn main() {
// All defaults flow from REST_PER_PAGE (100).
let page = PageNumberPagination { page_size: REST_PER_PAGE, max_page_size: None };
let flexible = PageNumberPagination { page_size: 20, max_page_size: Some(50) };
let limit_offset = LimitOffsetPagination { default_limit: 25, max_limit: 100 };
# let _ = (page, flexible, limit_offset);
# }
```

## Filter backends

`ViewSetConfig::filterable_fields` gives exact-match `?field=value` filtering.
Filter backends add lookup suffixes, free-text search, and client-controlled
ordering. They run in order and each one narrows the queryset further.

```rust,compile
# use djangors_rest::{FieldFilter, OrderingFilter, SearchFilter, ViewSetConfig, ViewSetOptions};
# use polls::models::Question;
# fn main() {
let options = ViewSetOptions::<Question>::new(ViewSetConfig::default())
    .with_filter_backend(FieldFilter::new(&["id", "pub_date"]))
    .with_filter_backend(SearchFilter::new(&["question_text"]))
    .with_filter_backend(OrderingFilter::new(&["pub_date"]));
# let _ = options;
# }
```

That accepts requests such as:

```text
?pub_date__gte=2026-01-01T00:00:00Z
?id__in=1,2,3
?question_text__icontains=rust
?search=rust
?ordering=-pub_date
```

Every backend is allowlist-driven. A parameter naming a field that was not
explicitly permitted is ignored rather than passed to SQL, and an unrecognised
lookup suffix is dropped the same way — a client cannot filter on a column the
endpoint never meant to expose.

### Writing your own backend (`FilterBackend`)

`FilterBackend<M>` is a plain trait; the three built-ins are just
implementations of it. Backends run in the order they were added, after the
`ViewSetConfig` exact-match allowlist, and each one narrows the queryset
further.

```rust,compile
use djangors_core::{Request, error::DjangorsError};
use djangors_orm::QuerySet;
use djangors_rest::FilterBackend;
use polls::models::Question;

// A backend that only ever shows questions published in the last 7 days.
struct RecentOnly;

impl FilterBackend<Question> for RecentOnly {
    fn filter_queryset(
        &self,
        _req: &Request,
        qs: QuerySet<Question>,
    ) -> Result<QuerySet<Question>, DjangorsError> {
        let week_ago = chrono::Utc::now() - chrono::Duration::days(7);
        qs.filter(djangors_orm::q!(pub_date__gte = week_ago))
            .map_err(|e| DjangorsError::Internal(e.to_string()))
    }
}
```

If you are not using a ViewSet at all, `djangors_rest::apply_backends(&backends,
&req, qs)` applies a slice of backends to a queryset by hand.

## Permissions

`AllowAny`, `IsAuthenticated`, `IsStaff`, `IsSuperuser`, and `IsReadOnly` ship
built in, and combine with `and` / `or` / `negate`:

```rust,compile
# use djangors_rest::{IsReadOnly, IsStaff, PermissionExt};
# fn main() {
// Staff may write; everyone else is limited to safe methods.
let permission = IsStaff.or(IsReadOnly);
# let _ = permission;
# }
```

## Throttling

`Throttle` applies a per-user (falling back to per-IP) rate limit to every
action on a ViewSet, using DRF's rate strings. It is backed by the same
cache-backed sliding window as `djangors_core::ratelimit`, so accounting is
best-effort under concurrency — the cache API has no atomic increment.

> [!WARNING]
> Constructor order is `(scope, rate, store)` — a name, a rate string, then the
> cache. Swapping the first two arguments (e.g. `("100/hour", "questions", …)`)
> compiles fine, because `scope` is just a `&'static str` and the rate string is
> also a `&'static str`; you only notice at runtime when the parse fails.
> `Throttle::new` returns `Option` (it delegates to `parse_rate`), so a typo'd
> rate is **silently `None`** unless you `.expect(...)`. Get the order right and
> always `expect`:

```rust,compile
# use std::sync::Arc;
# use djangors_cache::{Cache, InMemoryCache};
# use djangors_rest::Throttle;
# fn main() {
let store: Arc<dyn Cache> = Arc::new(InMemoryCache::new(10_000));
// Tuple-struct-style note: scope, then rate, then store.
let throttle = Throttle::new("questions", "100/hour", store).expect("valid rate");
# let _ = throttle;
# }
```

`parse_rate` accepts `second`, `minute`, `hour`, and `day` with their
abbreviations and plurals. A malformed rate returns `None` rather than silently
falling back to some default budget, so a typo is a configuration error you
catch at startup — but only because the example above calls `.expect`.

The `scope` (the first argument) isolates the budget: two endpoints at the same
rate sharing one cache do not consume each other's allowance.

`Throttle` plugs into a ViewSet via `ViewSetOptions::with_throttle`
(`viewset_routes_with_options`).

### Hand-rolled rate limiting (`RateLimiter`)

For a single endpoint that is not a full ViewSet — a login form, a contact
route, a webhook — use `djangors_core::RateLimiter` directly with `rate_limited`
to wrap the handler. Keys are chosen by a `RateLimitKey` strategy (`ByIp`, or
`ByAuthenticatedUser` / `ByUserOrIp` from `djangors-rest`).

```rust,compile
# use std::sync::Arc;
# use std::time::Duration;
# use djangors_core::{Router, Request, Response, PathParams, DjangorsError, StatusCode};
# use djangors_cache::InMemoryCache;
# async fn login(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
#     Ok(Response::text(StatusCode::OK, "ok"))
# }
# fn main() {
use djangors_core::{RateLimiter, ByIp, rate_limited};

let limiter = Arc::new(
    RateLimiter::new("login", ByIp, 5, Duration::from_secs(900),
        Arc::new(InMemoryCache::new(10_000))),
);

let router = Router::new().post("/login", rate_limited(limiter, login));
# let _ = router;
# }
```

A check that trips returns `DjangorsError::TooManyRequests`, so the wrapped
handler stays a plain `Result<Response, DjangorsError>`.

## Raw parse helpers: `deserialize` / `deserialize_partial`

When you are not using a ViewSet (a custom handler, an import endpoint), the
low-level parse helpers mirror DRF's serializer output shape: they map a JSON
object onto column values ready for `insert_raw` / `update`. They skip `auto`
fields, default missing booleans to `false`, and return a per-field `String`
map on failure.

- **`deserialize::<M>(&json)`** — full object; a missing non-nullable column is
  an error.
- **`deserialize_partial::<M>(&json)`** — missing keys are skipped, so it suits
  PATCH-style merges.

```rust,compile
# use polls::models::Question;
# use djangors_rest::deserialize;
# fn main() {
let json = serde_json::json!({ "question_text": "What is your name?" });
match deserialize::<Question>(&json) {
    Ok(values) => { /* values[0] = ("question_text", Value::Text("What is your name?")) */ }
    Err(errors) => { /* errors: HashMap<String, String>, field -> reason */ }
}
# }
```

## API authentication: `AuthToken`, `TokenAuth`, and JWT

`djangors-rest` authenticates API requests three ways; `current_user` and
`IsAuthenticated` try them in this order:

1. **Session** — `djangors-auth`'s `Auth<User>` (see the
   [Authentication guide](auth.md)). Best for browser clients.
2. **`Token` header** — `Authorization: Token <64-hex-key>`, backed by the
   database `AuthToken` model. Best for long-lived machine clients.
3. **JWT** — `Authorization: Bearer <jwt>`, HS256-signed, gated behind the
   crate's `jwt` Cargo feature. Best for short-lived cross-origin clients.

Issue a token with `generate_token_key()` and insert an `AuthToken` row, then
extract it in a handler with the `TokenAuth` extractor:

```rust,compile
# use djangors_rest::{generate_token_key, TokenAuth};
# use djangors_core::extract::{FromRequest};
# use djangors_core::{Request, DjangorsError};
# async fn create_token(db: &djangors_db::Database, user_id: i64) -> Result<(), Box<dyn std::error::Error>> {
use djangors_rest::AuthToken;
use djangors_orm::{Model, ForeignKey};

let key = generate_token_key();
AuthToken {
    id: 0,
    user: ForeignKey::new(user_id),
    key,
    created_at: chrono::Utc::now(),
}.save(db).await?;
# Ok(())
# }

# async fn use_token(req: Request) -> Result<(), DjangorsError> {
let TokenAuth(user) = TokenAuth::from_request(&req).await?;
# let _ = user;
# Ok(())
# }
```

### JWT

With the `jwt` feature enabled, `encode_jwt(user_id, secret)` produces a
one-hour HS256 token and `decode_jwt(token, secret)` recovers the user id. The
`JwtAuth` extractor parses the `Authorization: Bearer` header and resolves the
user from the database, mirroring `TokenAuth`:

```rust,compile
# use djangors_core::extract::FromRequest;
# use djangors_core::{Request, DjangorsError};
# use djangors_rest::JwtAuth;
# async fn use_jwt(req: Request) -> Result<(), DjangorsError> {
let JwtAuth(user) = JwtAuth::from_request(&req).await?;
# let _ = user;
# Ok(())
# }
```

```rust,compile
# fn main() {
use djangors_rest::encode_jwt;
let token = encode_jwt(42, "your-app-secret");
# let _ = token;
# }
```

## OpenAPI schema generation

`openapi_schema_for::<M>()` derives a JSON Schema object from a model's typed
metadata, and `OpenApiBuilder` aggregates models into a single OpenAPI document
with a title and version. Feed the result to Swagger UI / ReDoc.

```rust,compile
# use polls::models::Question;
# fn main() {
use djangors_rest::{OpenApiBuilder, openapi_schema_for};

let mut api = OpenApiBuilder::new("Polls API", "1.0.0");
api.register::<Question>("/api/questions");
let document = api.build();
# let _ = document;
# }
```

The schemas are derived from `FieldMeta`/`RelationMeta` — nullable fields are
optional, `max_length` becomes `maxLength`, and relations read as their integer
foreign-key id.

## Custom pagination strategies

The three built-ins (`PageNumberPagination`, `LimitOffsetPagination`,
`CursorPagination`) share the `Pagination` trait. Implement it yourself to send
a different envelope or read different query parameters:

```rust,compile
use djangors_core::{Request, Response, PathParams, DjangorsError};
use djangors_rest::{PageSlice, Pagination};

// A tiny "page + per_page" envelope mirroring the DRF DefaultPagination shape.
struct MyPagination {
    page_size: i64,
}

impl Pagination for MyPagination {
    fn slice(&self, req: &Request, total: i64) -> PageSlice {
        let page = req.query("page").and_then(|p| p.parse().ok()).unwrap_or(1).max(1);
        let offset = (page - 1) * self.page_size;
        PageSlice { limit: self.page_size, offset }
    }

    fn page_size(&self, req: &Request) -> i64 {
        self.page_size
    }

    fn envelope(&self, req: &Request, total: i64, results: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({
            "count": total,
            "page_size": self.page_size,
            "results": results,
        })
    }
}
```

Wire it in with `ViewSetOptions::with_pagination` inside
`viewset_routes_with_options`. `requested_page(&req)` is the helper the
built-in page-number pagination uses to read `?page=`.

## Error responses

Errors render as JSON when the request's `Accept` header asks for it, using a
stable envelope:

```json
{
  "error": {
    "code": "validation_error",
    "message": "Validation failed",
    "details": { "question_text": ["This field may not be blank."] }
  }
}
```

`code` is a stable, machine-readable string; `details` is omitted entirely when
there is nothing to report. Build one directly with
`DjangorsError::api(status, code, message)` and attach a payload with
`.with_details(json)`, or convert at a `?` site with the `ApiResultExt` helpers.
