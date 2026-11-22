# Forms & Form Processing

`djangors-forms` provides field validation, HTML widgets, layout renderers, and formsets, while `djangors-core` provides typed request body extraction for URL-encoded form POST submissions.

## The `Form<T>` Extractor

The `Form<T>` struct (`djangors_core::extract::Form`) extracts and deserializes form data submitted via HTTP `POST` requests (`application/x-www-form-urlencoded`).

```rust,compile
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, Request, Response, StatusCode};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct VoteForm {
    pub choice: i64,
}

pub async fn vote_handler(req: Request) -> Result<Response, DjangorsError> {
    // Deserialize URL-encoded request body bytes into VoteForm
    let Form(form) = Form::<VoteForm>::from_request(&req).await?;

    println!("Voted for choice ID: {}", form.choice);

    Ok(Response::text(StatusCode::OK, "Vote recorded"))
}
```

---

## Form Widgets & HTML Escaping

`djangors-forms` ships a suite of HTML form widgets implementing the [`Widget`](file:///root/dev/Rango/crates/djangors-forms/src/widgets.rs) trait. All widget output, including field names, values, option labels, and HTML attributes, is escaped against cross-site scripting (XSS).

Available widgets:
- `TextInput`: `<input type="text">`
- `Textarea`: `<textarea>`
- `NumberInput`: `<input type="number">`
- `CheckboxInput`: `<input type="checkbox">` (renders `checked` for boolean truthy values like `"true"`, `"on"`, `"1"`)
- `Select`: `<select>` with `<option>` choices
- `RadioSelect`: Radio inputs with label containers
- `DateInput`: `<input type="date">`
- `EmailInput`: `<input type="email">`
- `PasswordInput`: `<input type="password">` (by default, `render_value = false` to prevent echo of cleartext passwords)
- `HiddenInput`: `<input type="hidden">`

```rust,illustrative
use djangors_forms::{TextInput, PasswordInput, Select, Widget, WidgetAttrs};

let text_widget = TextInput;
let html = text_widget.render("username", Some("alice"), &WidgetAttrs::from([("class", "form-control")]));

let password_widget = PasswordInput::new(); // render_value = false by default
let pass_html = password_widget.render("password", Some("secret"), &WidgetAttrs::new());
```

---

## Form Layout Renderers (`as_div`, `as_table`, `as_p`)

Use [`BoundField`](file:///root/dev/Rango/crates/djangors-forms/src/renderers.rs) to bind fields to widgets, values, labels, help text, and errors, then render using layout helpers:

```rust,illustrative
use djangors_forms::{as_div, as_p, as_table, BoundField, TextInput};

let name_widget = TextInput;
let field = BoundField::new("first_name", &name_widget)
    .with_value(Some("Alice"))
    .with_help_text("Enter your given name")
    .with_errors(vec![]);

let div_html = as_div(&[field], &[]);
```

- `as_div`: Renders fields inside `<div>` containers.
- `as_p`: Renders fields inside `<p>` paragraphs.
- `as_table`: Renders fields as `<tr><th><label></th><td><widget></td></tr>` table rows.

### `BoundField` rendering helpers

Beyond the whole-layout helpers, a [`BoundField`](file:///root/dev/Rango/crates/djangors-forms/src/renderers.rs)
exposes granular renderers you can compose into custom layouts:

| Method | Returns | Notes |
| --- | --- | --- |
| `id_for_label()` | `String` | The element id (`attrs["id"]`, else `"id_<name>"`) |
| `label_tag()` | `String` | `<label for="...">…</label>` (sentence-cased name unless overridden) |
| `render_widget()` | `String` | The widget's HTML for this field |
| `render_errors()` | `String` | `<ul class="errorlist"><li>…</li></ul>` (empty when no errors) |
| `render_help_text()` | `String` | `<span class="helptext">…</span>` (empty when none) |

```rust,illustrative
use djangors_forms::{BoundField, TextInput};

let field = BoundField::new("first_name", &TextInput)
    .with_value(Some("Alice"))
    .with_help_text("Enter your given name")
    .with_errors(vec!["Too long.".to_string()]);

let id = field.id_for_label();        // "id_first_name"
let label = field.label_tag();        // <label for="id_first_name">First name</label>
let widget = field.render_widget();   // <input type="text" ... name="first_name" ...>
let errors = field.render_errors();   // <ul class="errorlist"><li>Too long.</li></ul>
let help = field.render_help_text();  // <span class="helptext">Enter your given name</span>
```

All output is HTML-escaped, including the field name, values, labels, and error/help text.

---

## FormSets & Security

[`FormSet`](file:///root/dev/Rango/crates/djangors-forms/src/formsets.rs) manages multiple instances of a form submitted in a single request.

Key features:
1. **Management Form**: Renders `TOTAL_FORMS` and `INITIAL_FORMS` as hidden inputs.
2. **Prefixing**: Prefixing field names per form (`form-0-title`, `form-1-title`, ...).
3. **Deletion**: `can_delete` option renders a `DELETE` checkbox per form.
4. **Security Cap**: `TOTAL_FORMS` is checked against `max_num` (default `1000`) before allocation to prevent memory-exhaustion attacks.

```rust,illustrative
use djangors_forms::{FormSet, FormSetFormResult};
use std::collections::HashMap;

let formset: FormSet<()> = FormSet::new()
    .with_prefix("author")
    .with_counts(2, 1)
    .with_can_delete(true);

let mgmt_form_html = formset.render_management_form();

let mut post_data = HashMap::new();
post_data.insert("author-TOTAL_FORMS".to_string(), "1".to_string());
post_data.insert("author-INITIAL_FORMS".to_string(), "0".to_string());
post_data.insert("author-0-name".to_string(), "Jane Doe".to_string());

let results = formset.clean_with(&post_data, |map| {
    Ok(map.get("name").cloned().unwrap_or_default())
});
```

### FormSet field-naming & rendering helpers

| Method | Signature | Notes |
| --- | --- | --- |
| `with_max_num(max)` | `fn(self, usize) -> Self` | Raises/lowers the `TOTAL_FORMS` security cap (default `1000`) |
| `add_prefix(index, field)` | `fn(&self, usize, &str) -> String` | Builds `"<prefix>-<index>-<field>"` (e.g. `author-0-title`) |
| `render_delete_checkbox(index, checked)` | `fn(&self, usize, bool) -> String` | Renders the `<input type="checkbox" name="...-DELETE">` (empty unless `can_delete`) |

```rust,illustrative
use djangors_forms::FormSet;

let formset: FormSet<()> = FormSet::new()
    .with_prefix("author")
    .with_counts(2, 1)
    .with_max_num(5)                 // cap TOTAL_FORMS before allocation (default 1000)
    .with_can_delete(true);

let name = formset.add_prefix(0, "title");             // "author-0-title"
let delete_box = formset.render_delete_checkbox(0, false); // <label ...>… Delete</label>
```

`add_prefix` is what you should use when reading individual prefixed fields back from submitted
data, and `render_delete_checkbox` returns an empty string unless `can_delete` is enabled — so
templates can call it unconditionally.

---

## How Form Extraction Works

1. `Form<T>::from_request(&req).await` reads the full request body bytes via `req.body_bytes().await`.
2. Body bytes are deserialized into type `T` (which must implement `serde::de::DeserializeOwned`) using `serde_urlencoded::from_bytes`.
3. **Error Handling**:
   - If the request body contains missing required fields or invalid data types for `T`, extraction returns `DjangorsError::BadRequest(msg)` containing `"failed to parse form body: ..."` with HTTP status `400 Bad Request`.

---

## Combining Extractors in Handlers

`djangors-core` extractors implement `FromRequest`:

```rust,compile
use djangors_core::extract::{Form, FromRequest, Json, Query};
```

| Extractor | Source | Content Type / Format |
|---|---|---|
| `Form<T>` | Request Body | `application/x-www-form-urlencoded` |
| `Json<T>` | Request Body | `application/json` |
| `Query<T>` | URI Query String | `?key=value&...` |

### Path Parameter Extraction
Path parameters are retrieved using `extract_path_param`:

```rust,compile
# use djangors_core::{Request, PathParams, Response, DjangorsError};
use djangors_core::extract::extract_path_param;

pub async fn detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let question_id: i64 = extract_path_param(&params, "id")?;
    let _ = (req, question_id);
    Ok(Response::text(djangors_core::StatusCode::OK, ""))
}
```

---

## The `#[derive(Form)]` validation macro

For validation on top of plain `Deserialize`, `#[derive(Form)]` generates a
`clean(&HashMap<String, String>)` method with per-field rules declared in
attributes. Unlike `Deserialize` it runs your rules and returns a structured
`FormErrors` (per-field messages) instead of a single parse error.

```rust,compile
#[derive(djangors_forms::Form, Debug)]
struct SignupForm {
    #[djangors(max_length = 150, required = true)]
    username: String,

    #[djangors(email, required = true)]
    email: String,

    #[djangors(min = 13)]
    age: i64,
}

# #[allow(dead_code)]
fn run(data: &std::collections::HashMap<String, String>) {
    match SignupForm::clean(data) {
        Ok(cleaned) => { println!("email={}", cleaned.email); }
        Err(errors) => { /* errors.fields: HashMap<String, FieldError> */ }
    }
}
```

Supported field attributes: `max_length` + `required = true` + `email` on
`String`, `min`/`max`/`required = true` on `i64`/`i32`, `required = true` on
`bool`. (For `required`, the value form with `= true` is required — a bare
`required` flag is not accepted.) The generated `{Form}Cleaned` struct's
visibility mirrors the form's own.

---

## Model-generated forms (`validate_form` / `apply_cleaned_form`)

`#[derive(Model)]` also generates a real Django-`ModelForm`-equivalent directly
on the model — no second struct required. Auto/PK fields and `FileField`s are
excluded automatically, exactly like `save()`'s `INSERT`.

```rust,compile
# use djangors_macros::Model;
# use djangors_orm::Model;
# use std::collections::HashMap;
# #[derive(Model, Debug, Clone)]
# #[djangors(app = "library", table_name = "library_contact")]
# struct Contact { #[djangors(primary_key, auto)] id: i64, #[djangors(max_length = 100)] name: String, email: String }
# #[allow(dead_code)]
async fn insert_contact(data: HashMap<String, String>, db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
    // `validate_form` returns `Result<FormCleaned, FormErrors>`; FormErrors is a
    // value (field-keyed messages), not a std::error::Error — match on it and
    // re-render the form with `errors.fields` rather than using `?`.
    let contact = match Contact::validate_form(&data) {
        Ok(cleaned) => Contact::from_cleaned_form(cleaned), // fresh instance
        Err(_errors) => return Ok(()),                      // re-render with errors.fields
    };
    contact.save(db).await?; // INSERT
    Ok(())
}
```

The three generated methods map onto the create and update paths:

| Method | Create path | Update path |
| --- | --- | --- |
| `validate_form(&map)` | Validate the post body | Same, against the same fields |
| `from_cleaned_form(cleaned)` | Build a **new** instance, then `save()` | — |
| `apply_cleaned_form(cleaned)` | — | Apply onto an **existing** instance (leaving the PK untouched), then `update()` |

`apply_cleaned_form` takes a `&mut self`, so the update path is: fetch the row,
`validate_form` the submitted data, `apply_cleaned_form`, then `update(db)` —
exactly the shape `djangors-views`' `UpdateView` implements for you.

Use with `djangors-views`' generic `CreateView`/`UpdateView` (see the
[Class-Based Views guide](class-based-views.md)) to get a full
form-render-and-submit flow without writing a handler by hand.

---

## Djangors 1.0 Field Types, Widgets, Media & Inline FormSets

### Built-in Form Field Types

In addition to `CharField`, `IntegerField`, `BooleanField`, `EmailField`, and `ChoiceField`, Djangors 1.0 ships 17 new `FormField` types in `djangors_forms`:

| Field Type | Parsed / Cleaned Value | Key Validations |
| --- | --- | --- |
| `FloatField` | `f64` | Rejects NaN/Inf; optional `min_value` / `max_value` |
| `DecimalField` | `rust_decimal::Decimal` | `max_digits`, `decimal_places`, min/max bounds |
| `DateField` | `chrono::NaiveDate` | Parses ISO `YYYY-MM-DD` (and custom formats) |
| `DateTimeField` | `chrono::NaiveDateTime` / `DateTime` | Parses ISO-8601 and HTML `datetime-local` strings |
| `TimeField` | `chrono::NaiveTime` | Parses `HH:MM` and `HH:MM:SS` |
| `DurationField` | `chrono::Duration` | Parses ISO-8601 or `HH:MM:SS` / seconds |
| `UrlField` | `String` | Validates `http://` / `https://` scheme and host |
| `SlugField` | `String` | Allows ASCII letters, digits, hyphens, and underscores |
| `UuidField` | `uuid::Uuid` | Validates canonical UUID strings |
| `FileField` | `String` / upload metadata | Optional `max_length` and allowed extensions |
| `ImageField` | `String` / image upload metadata | Validates image MIME/extension allowlist |
| `MultipleChoiceField` | `Vec<String>` | Ensures every submitted choice is in `choices` |
| `TypedChoiceField<T>` | `T` | Validates choice membership and coerces via custom parser |
| `JsonField` | `serde_json::Value` | Parses valid JSON payloads |
| `GenericIpAddressField` | `std::net::IpAddr` | Configurable `IpProtocol::{Both, IPv4, IPv6}` |
| `ModelChoiceField` | `i64` / PK string | Validates against an allowed set of model primary keys |
| `ModelMultipleChoiceField` | `Vec<i64>` / PK list | Validates multiple selected model primary keys |

```rust,illustrative
use djangors_forms::{
    DateField, DecimalField, FormField, GenericIpAddressField, IpProtocol, JsonField, SlugField,
    UrlField, UuidField,
};

let slug = SlugField::new().clean("hello-djangors-1-0")?;
let url = UrlField::new().clean("https://djangors.dev")?;
let ip = GenericIpAddressField::new(IpProtocol::Both).clean("2001:db8::1")?;
let payload = JsonField::new().clean(r#"{"active": true}"#)?;
let date = DateField::new().clean("2026-07-30")?;
```

### New Widgets & `FormMedia`

Djangors 1.0 adds 8 new HTML widgets (all XSS-escaped):

| Widget | Rendered HTML |
| --- | --- |
| `FileInput` | `<input type="file">` |
| `ClearableFileInput` | File input plus current file link and clear checkbox |
| `SelectMultiple` | `<select multiple>` with multi-value selection |
| `CheckboxSelectMultiple` | List of `<input type="checkbox">` options |
| `DateTimeInput` | `<input type="datetime-local">` |
| `TimeInput` | `<input type="time">` |
| `UrlInput` | `<input type="url">` |
| `SplitDateTimeWidget` | Paired `<input type="date">` and `<input type="time">` inputs |

Use **`FormMedia`** (`djangors_forms::FormMedia`) to declare, merge, and render CSS/JS assets required by custom widgets or forms:

```rust,illustrative
use djangors_forms::FormMedia;

let media = FormMedia::new()
    .add_css("all", "/static/widgets/datepicker.css")
    .add_js("/static/widgets/datepicker.js");

let combined = media.merge(&FormMedia::new().add_js("/static/app.js"));
let head_tags = combined.render(); // <link rel="stylesheet" ...> and <script src="..."></script>
```

### `ManagementForm` & `InlineFormSet`

- **`ManagementForm`** models and validates the hidden `TOTAL_FORMS`, `INITIAL_FORMS`, `MIN_NUM_FORMS`, and `MAX_NUM_FORMS` fields for any `FormSet`.
- **`InlineFormSet`** wraps a `FormSet` bound to a parent record's foreign-key field (`parent_fk_name`) and primary key (`parent_pk`), automatically injecting the parent FK into every child form during cleaning:

```rust,illustrative
use djangors_forms::{FormSet, InlineFormSet};
use std::collections::HashMap;

let inline: InlineFormSet<()> = InlineFormSet::new(
    FormSet::new().with_prefix("items").with_can_delete(true),
    "order_id",
    "42",
);

let mgmt_html = inline.formset.render_management_form();
let cleaned = inline.clean_with(&post_data, |child_map| {
    // `child_map["order_id"]` is automatically populated with `"42"`
    Ok(child_map.get("sku").cloned().unwrap_or_default())
})?;
```
