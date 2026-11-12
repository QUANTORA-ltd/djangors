# Admin Site

`djangors-admin` provides an automatic, customizable administration interface for managing database models.

## `AdminSite` Initialization & Branding

Create and configure an `AdminSite` instance:

```rust,compile
# fn main() {
use djangors_admin::{AdminSite, ModelAdminConfig};
use polls::models::{Choice, Question};

let admin = AdminSite::new()
    .with_site_header("Polls Administration")
    .with_site_title("Polls Admin")
    .with_logo_url("/static/logo.png")
    .with_accent_color("#2563eb");

// Register models
admin.register::<Question>();
admin.register_with::<Choice>(ModelAdminConfig {
    list_display: Some(&["choice_text", "votes", "question"]),
    search_fields: Some(&["choice_text"]),
    list_filter: Some(&[]),
    ..Default::default()
});
# }
```

---

## `ModelAdminConfig` Options

Customize how models are displayed and edited in the admin interface:

```rust,compile
# use djangors_admin::AdminAction;
pub struct ModelAdminConfig {
    pub list_display: Option<&'static [&'static str]>,
    pub search_fields: Option<&'static [&'static str]>,
    pub list_filter: Option<&'static [&'static str]>,
    pub date_hierarchy: Option<&'static str>,
    pub list_editable: Option<&'static [&'static str]>,
    pub computed_columns: Option<
        &'static [(
            &'static str,
            fn(&[(&'static str, djangors_orm::expr::Value)]) -> String,
        )],
    >,
    pub actions: Option<&'static [AdminAction]>,
    pub fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    pub readonly_fields: Option<&'static [&'static str]>,
    pub raw_id_fields: Option<&'static [&'static str]>,
    pub base_filter: Option<djangors_orm::UnresolvedExpr>,
}
```

### Config Field Details
- **`list_display`**: Fields to render as columns on the changelist page.
- **`search_fields`**: Text fields to `ILIKE`-search against `?q=` queries.
- **`list_filter`**: Boolean fields to render filter options in the sidebar.
- **`date_hierarchy`**: DateTime field for date drill-down navigation.
- **`list_editable`**: Fields editable inline directly from the changelist table.
- **`computed_columns`**: Custom display column functions evaluated dynamically per row.
- **`actions`**: Bulk actions callable on selected rows (a `"delete_selected"` bulk action is registered by default).
- **`fieldsets`**: Groups fields into titled sections on the add/change form.
- **`readonly_fields`**: Prevents editing specified fields in forms.
- **`raw_id_fields`**: Renders foreign key inputs as raw integer ID fields with target lookup links.
- **`base_filter`**: Baseline queryset filter automatically applied to all changelist queries.

> [!IMPORTANT]
> `register_with` **validates your config at registration time, at startup, by
> `panic`** — not at first request. A field named in `list_display`,
> `search_fields`, `list_filter`, `date_hierarchy`, or `list_editable` that does
> not exist on the model, or that has the wrong kind (a `String` in
> `list_filter`, which must be Boolean; a `ForeignKey`/integer in
> `search_fields`, which must be text-like; a non-`DateTime` in
> `date_hierarchy`), produces a hard panic the moment the admin site is
> registered. Treat the config as compile-time-ish: a wrong type surfaces as an
> immediate startup crash, which is precise but not graceful. The allowed kinds
> are: `list_filter` → Boolean only; `search_fields` → `Char`/`Text`/`Email`/
> `Url`/`Slug`/`Ip`; `list_editable` → text and numeric kinds; `date_hierarchy`
> → `DateTime`.

---

## Favicons & Static Branding Routes

`djangors-admin` includes static favicon serving helper `favicon_routes`:

```rust,compile
# fn main() {
use djangors_admin::favicon_routes;
use djangors_core::Router;

let router = favicon_routes(Router::new());
# }
```

Routes served:
- `/favicon.ico`
- `/favicon-16x16.png`
- `/favicon-32x32.png`
- `/apple-touch-icon.png`
- `/android-chrome-192x192.png`
- `/android-chrome-512x512.png`
- `/manifest.json`

---

## Audit Logging (`LogEntry`)

All admin actions (additions, updates, deletions) are recorded in the `djangors_admin_log` table via `LogEntry`:
- **`ACTION_ADDITION`** (`1`): Object creation.
- **`ACTION_CHANGE`** (`2`): Object field edits (with field diff payload).
- **`ACTION_DELETION`** (`3`): Object removal.

---

## Computed columns & `base_filter`

`computed_columns` lets a changelist column be produced by a function instead of
a raw model field. The function receives the row's field/value pairs and returns
the display string; list it in `list_display` alongside real fields.

```rust,compile
# use djangors_admin::{AdminSite, ModelAdminConfig};
# use djangors_orm::expr::Value;
# use polls::models::Question;
# fn main() {
// A column computed from two fields (`id` and `question_text`).
fn id_and_text(values: &[(&'static str, Value)]) -> String {
    let text = values
        .iter()
        .find(|(n, _)| *n == "question_text")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    format!("#{} {text}", values.iter().find(|(n, _)| *n == "id").map(|(_, v)| v.to_string()).unwrap_or_default())
}

let site = AdminSite::new();
site.register_with::<Question>(ModelAdminConfig {
    list_display: Some(&["id", "combo"]),
    computed_columns: Some(&[("combo", id_and_text)]),
    ..Default::default()
});
# }
```

`base_filter` applies an `UnresolvedExpr` (the same value `q!` produces) to
every changelist query — Django's `get_queryset`-scoping for the admin list.
Combine it with [`Scoped`](rest.md) semantics when a multi-tenant admin exists,
or simply to default a soft-deleted/model-state filter:

```rust,compile
# use djangors_admin::{AdminSite, ModelAdminConfig};
# use polls::models::Question;
# fn main() {
let site = AdminSite::new();
site.register_with::<Question>(ModelAdminConfig {
    // Only list questions with any votes (q! result used verbatim).
    base_filter: Some(djangors_orm::q!(votes__gt = 0i32) as djangors_orm::UnresolvedExpr),
    ..Default::default()
});
# }
```

> [!NOTE]
> `q!` returns `UnresolvedExpr` already, so no cast is needed in real code; the
> explicit `as` above only illustrates the expected type of the field.

## The `ModelAdmin` trait

`register_with(ModeAdminConfig)` covers the common cases; for full control,
implement the `ModelAdmin` trait directly and register it off the standard
config flow. The required methods describe the model (`model_meta`,
`field_names`, `changelist`, `export_csv_rows`, `get_by_pk`,
`update_from_form`/`create_from_form`, `delete_by_pk`), and the optional ones
default when omitted (`actions`, `fieldsets`, `readonly_fields`,
`raw_id_fields`, plus the search/filter/editable/hierarchy accessors).
`DefaultModelAdmin<M>` in `djangors-admin` is the built-in implementation
`register_with` uses, and you can use it as a reference when writing your own.

---

## Crate layout

`djangors-admin` is organized by view rather than by model. `lib.rs` holds only the
crate-level docs, the shared template engine (`ADMIN_TEMPLATES`), the module list, and the
public re-exports — so the public API you `use` is identical regardless of which module an
item lives in. A contributor can find things like this:

| Module | Owns |
| --- | --- |
| `model_admin` | `ModelAdmin` trait, `DefaultModelAdmin`, `ModelAdminConfig`, `ChangelistPage`, `InlineConfig`, `TenantFilter` |
| `site` | `AdminSite`, `SiteBranding`, `TenantScope`, `with_tenant_scoping`, route assembly (`urls`) |
| `changelist` | List page: querying, pagination, CSV export, `?o=/`q=/`f=`/date-hierarchy handling |
| `edit_view` | Add/change form GET+POST handlers |
| `delete_view` | Delete and bulk-delete GET+POST handlers |
| `index_view` | The admin index (app/model links) |
| `history_view` | Per-object change history |
| `action_view` | Bulk-action handlers |
| `forms` | Form/field rendering and parsing for the add/change forms |
| `log` | `LogEntry` auditing (`AdminAction`, field diffs) |
| `perms` | `require_staff`, `require_perm`, and request-scoped tenant resolution |
| `favicon` | `favicon_routes` static branding routes |

`site.rs` depends on the view modules; the view modules depend on the `ModelAdmin` trait
defined in `model_admin.rs`. `lib.rs` re-exports the public surface
(`AdminSite`, `SiteBranding`, `TenantScope`, `ModelAdmin`, `DefaultModelAdmin`,
`ModelAdminConfig`, `InlineConfig`, `TenantFilter`, `ChangelistPage`, `AdminAction`,
`LogEntry`, `favicon_routes`, ...), so the module an item lives in is an implementation
detail rather than part of the public API.

---

## Implementing `ModelAdmin` by hand

The section above introduced the trait. Whatever you implement, the framework hands you a
`TenantFilter` — `Option<(&'static str, i64)>`, the tenant FK field name on the model plus the
tenant id every row must match — as the **final argument** to every method that touches data:
`changelist`, `export_csv_rows`, `get_by_pk`, `update_from_form`, `update_fields_from_form`,
`create_from_form`, and `delete_by_pk`. Their real signatures:

```rust,illustrative
use djangors_admin::{ChangelistPage, ModelAdmin, TenantFilter};

async fn changelist(
    &self,
    db: &djangors_db::Database,
    order: Option<&str>,
    page: i64,
    per_page: i64,
    search: Option<&str>,
    filters: &[(&'static str, bool)],
    date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    tenant: TenantFilter,
) -> Result<ChangelistPage, djangors_core::DjangorsError>;

async fn export_csv_rows(
    &self,
    db: &djangors_db::Database,
    order: Option<&str>,
    search: Option<&str>,
    filters: &[(&'static str, bool)],
    date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
    tenant: TenantFilter,
) -> Result<(Vec<&'static str>, Vec<Vec<String>>), djangors_core::DjangorsError>;

async fn get_by_pk(
    &self,
    db: &djangors_db::Database,
    pk: i64,
    tenant: TenantFilter,
) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, djangors_core::DjangorsError>;

async fn update_from_form(
    &self,
    db: &djangors_db::Database,
    pk: i64,
    form: &std::collections::HashMap<String, String>,
    tenant: TenantFilter,
) -> Result<Result<(), std::collections::HashMap<String, String>>, djangors_core::DjangorsError>;

async fn update_fields_from_form(
    &self,
    db: &djangors_db::Database,
    pk: i64,
    form: &std::collections::HashMap<String, String>,
    tenant: TenantFilter,
) -> Result<Result<(), std::collections::HashMap<String, String>>, djangors_core::DjangorsError>;

async fn create_from_form(
    &self,
    db: &djangors_db::Database,
    form: &std::collections::HashMap<String, String>,
    tenant: TenantFilter,
) -> Result<Result<i64, std::collections::HashMap<String, String>>, djangors_core::DjangorsError>;

async fn delete_by_pk(
    &self,
    db: &djangors_db::Database,
    pk: i64,
    tenant: TenantFilter,
) -> Result<bool, djangors_core::DjangorsError>;
```

The one thing a custom implementation must get right is this contract:

> **Apply the `tenant` argument to every query you run, and never cache it on `self`.**

The reason is structural. The admin object is built once at registration and shared across
every concurrent request behind an `Arc`; there is no per-request copy. If you stored the
resolved tenant on the admin, a second request could overwrite it while the first awaited the
database, and one tenant's request would then read another tenant's rows. So the tenant always
arrives per-call, and it is up to your method to thread it into the query. `None` means
unrestricted — a superuser, or a site with no tenant scoping configured at all.

The PK-based methods carry the same obligation with specific semantics, and the safe default
is to scope by both PK and tenant like `DefaultModelAdmin` does:

- **`get_by_pk`** must read a row belonging to another tenant as "not found" (`None`), not
  return it — a cross-tenant primary key and a missing one are indistinguishable.
- **`delete_by_pk`** must refuse (return `false`, delete nothing) when the row belongs to
  another tenant.
- **`update_from_form` / `update_fields_from_form`** must not touch another tenant's row.
- **`create_from_form`** must force the new row's tenant field to the resolved tenant rather
  than trusting the submitted form, so a scoped add-form can never plant a row elsewhere.

The default build does all of this via `tenant_compare` plus PK-scoped querysets, and the
[Multi-tenancy guide](multi-tenancy.md) walks through the reasoning, the "not stored on the
admin" design, and the fail-closed resolution rules in full — read it before writing a custom
`ModelAdmin` for a scoped site.

