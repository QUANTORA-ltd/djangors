# Object-Relational Mapping (ORM)

`djangors-orm` provides Django-style querysets, model metadata, expression macros, compile-time JSON writers, and database integration for PostgreSQL and SQLite (via `sqlx`).

## Defining Models

Models are Rust structs decorated with `#[derive(Model)]` from `djangors-macros`.

```rust,compile
# use djangors_orm::Model;
use djangors_macros::Model;
use djangors_orm::ForeignKey;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_question", ordering = ["-pub_date"])]
pub struct Question {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(max_length = 200)] pub question_text: String,

    pub pub_date: DateTime<Utc>,
}

#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_choice")]
pub struct Choice {
    #[djangors(primary_key, auto)]
    pub id: i64,

    #[djangors(foreign_key(on_delete = "cascade", related_name = "choices"))]
    pub question: ForeignKey<Question>,

    #[djangors(max_length = 200)]
    pub choice_text: String,

    #[djangors(default = 0)]
    pub votes: i32,
}
```

### Model Attributes (`#[djangors(...)]`)

#### Struct-Level Attributes
- **`app = "app_label"`** *(required)*: Specifies the application label (e.g., `"polls"`).
- **`table_name = "custom_table"`** *(optional)*: Database table name. Defaults to `{app_label}_{snake_case_struct_name}`.
- **`ordering = ["field", "-field"]`** *(optional)*: Default ordering for querysets when `.order_by()` is not called. Prefix `-` denotes descending.
- **`unique_together = [["field1", "field2"]]`** *(optional)*: Defines composite unique constraints.
- **`indexes = ["index_name", ("index_name", ["field1", "field2"])]`** *(optional)*: Defines custom or composite database indexes. Can be a simple index name or a tuple of `(name, [fields])`.
- **`manager = "ManagerStruct"`** *(optional)*: Overrides the default model manager with a custom manager struct.
- **`soft_delete`** *(optional)*: Makes deletes soft: rows get a `deleted_at` timestamp instead of being removed, and normal queries hide them. The struct must have a `deleted_at: Option<DateTime<Utc>>` field. See [Soft deletes](#soft-deletes).

#### Field-Level Attributes
- **`primary_key`**: Marks the single primary key field. Automatically implies `unique` and `db_index`. Every model must have exactly one primary key field.
- **`auto`**: Marks auto-incrementing / database-generated fields (such as `BIGSERIAL`). Ignored during `save()` insertions.
- **`max_length = N`**: Sets maximum string length. Valid only on `String` fields (maps to `FieldKind::Char`). `String` without `max_length` maps to `FieldKind::Text`.
- **`default = value`**: Sets default field value (`Text`, `I64`, `F64`, or `Bool`).
- **`unique`**: Adds a `UNIQUE` constraint to the column.
- **`db_index`**: Creates a database index on the column.
- **`verbose_name = "..."`**, **`help_text = "..."`**: Descriptive labels for admin/forms metadata.
- **`column = "col_name"`**: Overrides the database column name if different from the struct field name.
- **`auto_now_add`**: Populates a `DateTime<Utc>` field with `Utc::now()` automatically when a new row is inserted via `save(db)`.
- **`auto_now`**: Refreshes a `DateTime<Utc>` field with `Utc::now()` automatically on both `save(db)` and `update(db)`.
- **`choices = [("VALUE", "Human Label"), ...]`**: Restricts a `String` field to an allowed set of values (validated in forms/serializers and emitted as a SQL `CHECK` constraint by migrations).
- **`foreign_key(on_delete = "...", related_name = "...")`**: Configures foreign key behavior. `on_delete` options: `"cascade"` (default), `"protect"`, `"set_null"`, `"restrict"`, `"do_nothing"`.

### Supported Field Types
- Integer: `i32` (`FieldKind::Integer`), `i64` (`FieldKind::BigInt`)
- Float: `f32`, `f64` (`FieldKind::Float`)
- Decimal: `rust_decimal::Decimal` (`FieldKind::Decimal { max_digits, decimal_places }`)
- String: `String` (`FieldKind::Char` with `max_length`, or `FieldKind::Text` without)
- Boolean: `bool` (`FieldKind::Boolean`)
- Datetime & Date/Time: `chrono::DateTime<chrono::Utc>` (`FieldKind::DateTime`), `chrono::NaiveDate` (`FieldKind::Date`), `chrono::NaiveTime` (`FieldKind::Time`), `chrono::Duration` (`FieldKind::Duration`)
- UUID: `uuid::Uuid` (`FieldKind::Uuid`)
- Foreign Key: `ForeignKey<TargetModel>`
- One-to-One Key: `OneToOne<TargetModel>`
- Many-to-Many Marker: `ManyToMany<TargetModel>`
- Nullable: `Option<T>` for any of the above types (except `ManyToMany`)

### Custom Managers

You can define custom database query manager behavior by implementing the `Manager` trait and specifying the manager class via the struct-level `manager` attribute:

```rust,compile
# use djangors_orm::{Model, q};
# use djangors_macros::Model;
pub struct ActiveManager;

impl djangors_orm::meta::Manager<ManagedModel> for ActiveManager {
    fn objects() -> djangors_orm::queryset::QuerySet<ManagedModel> {
        djangors_orm::queryset::QuerySet::<ManagedModel>::new()
            .filter(q!(is_active = true))
            .expect("valid filter")
    }
}

#[derive(Model, Debug)]
#[djangors(
    app = "polls",
    table_name = "polls_managed",
    manager = "ActiveManager"
)]
pub struct ManagedModel {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    pub is_active: bool,
}
```

By overriding the manager, calling `ManagedModel::objects()` will automatically retrieve a queryset pre-filtered or configured according to your manager definition.

---

## QuerySet Methods

Access querysets using `Model::objects()` (or `QuerySet::<T>::new()`).

> [!IMPORTANT]
> `objects()`, `meta()`, and `field_values()` are methods on the **`Model`
> trait**. Every file that calls them needs `use djangors_orm::Model;` — the
> `#[derive(Model)]` attribute does not pre-import it. A bare
> `Model::objects()` failing with "method not found" is this missing import, not
> a broken model.

### Filtering and Ordering
- **`.filter(q!(field = value, ...))`**: Applies filter expressions combined with `AND`. Accepts a whole `Q`-style tree, so `OR` and `NOT` compose (see [Combining filters](#combining-filters-or-and-not)).
- **`.exclude(q!(...))`**: The negation of `.filter()` — Django's `.exclude()`.
- **`.filter_or_icontains(&["field1", "field2"], "term")`**: Generates case-insensitive `ILIKE %term%` queries across specified text fields combined with `OR`.
- **`.filter_datetime_range(field, gte, lt)`**: Filters datetime fields in the half-open interval `[gte, lt)`.
- **`.order_by("field")` / `.order_by("-field")`**: Orders results. Prefix `-` means `DESC`.
- **`.limit(n)` / `.offset(n)`**: Paginates results using standard SQL `LIMIT` and `OFFSET`.
- **`.debug_sql()` / `.debug_params()`**: The `SELECT` this queryset would run and the parameters it would bind, for debugging and tests — Django's `str(queryset.query)`. Placeholders stay as `$1`, `$2`, … rather than being interpolated, so the output is never a runnable statement.

### Lookup Suffixes

Append a lookup to a field name to change the comparison. Without one, `=` is used.

| Suffix | SQL | Notes |
| --- | --- | --- |
| *(none)* / `__eq` | `=` | |
| `__ne` | `<>` | |
| `__lt` `__lte` `__gt` `__gte` | `<` `<=` `>` `>=` | |
| `__contains` / `__icontains` | `LIKE` / `ILIKE` `%v%` | |
| `__startswith` / `__endswith` | `LIKE 'v%'` / `LIKE '%v'` | |
| `__iexact` | `ILIKE 'v'` | Case-insensitive, no wildcards |
| `__in` | `IN (...)` | Takes a `Vec`; an empty one compiles to `FALSE`, never invalid `IN ()` |
| `__isnull` | `IS NULL` / `IS NOT NULL` | `= false` inverts it; binds no parameter |
| `__regex` / `__iregex` | `~` / `~*` | POSIX regular expression |

An unrecognised suffix is treated as part of the field name, so a typo surfaces
as `OrmError::FieldNotFound` rather than silently becoming an equality test.

### Execution Methods
- **`.all(db).await`**: Executes select query and returns `Vec<T>`.
- **`.get(db).await`**: Fetches a single object. Returns `Ok(T)`, `Err(OrmError::NotFound)` if 0 rows returned, or `Err(OrmError::MultipleObjectsReturned)` if >1 row.
- **`.first(db).await`**: Returns `Result<Option<T>, OrmError>` (`LIMIT 1`).
- **`.exists(db).await`**: Returns `Result<bool, OrmError>` check (`SELECT 1 ... LIMIT 1`).
- **`.count(db).await`**: Returns `Result<i64, OrmError>` count (`SELECT COUNT(*) ...`).
- **`.aggregate(db, vec![AggExpr::...]).await`**: Evaluates aggregate functions: `AggExpr::Count`, `AggExpr::Sum`, `AggExpr::Avg`, `AggExpr::Min`, `AggExpr::Max`.
- **`.update(db, set!(...)).await`**: Performs bulk updates. Returns `Result<u64, OrmError>` (rows affected count).
- **`.delete(db).await`**: Deletes all rows matching the queryset's filters. Returns `Result<u64, OrmError>` (rows affected count). On a soft-delete model it sets `deleted_at` instead (see [Soft deletes](#soft-deletes)). **Warning**: calling `.delete()` on an unfiltered queryset (e.g. `Model::objects().delete(db)`) will delete every row in the table.
- **`QuerySet::<T>::delete_by_pk(db, pk).await`**: Static helper deleting a row by its primary key. On a soft-delete model it sets `deleted_at`; `force_delete_by_pk` always removes the row.

### Eager-loading relations: `select_related` and `prefetch_related`

`select_related` loads a forward foreign key in **two queries total** (batch the
related ids, then one `IN` fetch), instead of one query per row. It returns
`Vec<(T, Option<R>)>` — the row, plus the related model or `None` when the
foreign key is dangling.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{Model, prefetch_related};
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
// One query for choices, one for their questions — not N+1.
let rows: Vec<(Choice, Option<Question>)> = Choice::objects()
    .select_related::<Question, _>(db, "question")
    .await?;

for (choice, question) in &rows {
    let text = question.as_ref().map(|q| q.question_text.as_str()).unwrap_or("[missing]");
    println!("{} -> {text}", choice.choice_text);
}
# Ok(())
# }
```

`prefetch_related` does the reverse direction: given a list of parent rows, it
batch-loads the children that FK to them in one query. It is a free function —
call it with the parents and the `related_name` (needs [`ForeignKey`](#relationships--many-to-many)
to declare one).

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{Model, prefetch_related, q};
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let questions = Question::objects().all(db).await?;

// One extra query fills in every question's `choices` (related_name).
let by_question: std::collections::HashMap<i64, Vec<Choice>> =
    prefetch_related(db, &questions, "choices").await?;

for question in &questions {
    println!("{0}: {1} choices", question.question_text, by_question.get(&question.id).map_or(0, Vec::len));
}
# Ok(())
# }
```

Use `select_related` for `-to-one` hops and `prefetch_related` for `-to-many`
reversed relations; both exist purely to avoid the N+1 query pattern.

### Locking rows: `select_for_update`

`.filter(...).select_for_update()` prefixes the query with `FOR UPDATE`, so the
matched rows are locked until the transaction commits. Combined with
`.nowait()` or `.skip_locked()` it implements the "claim one of N jobs" pattern
without double-claiming.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{Model, q};
# async fn run(db: &djangors_db::Database, choice_id: i64) -> Result<(), Box<dyn std::error::Error>> {
db.transaction(|conn| {
    Box::pin(async move {
        let choice: Choice = Choice::objects()
            .filter(q!(id = choice_id))?   // note: no `.await` — still building the query
            .select_for_update()
            .get(conn)
            .await?;
        // `choice` is locked for this transaction; other transactions block or skip.
        Result::<(), djangors_orm::OrmError>::Ok(())
    })
}).await?;
# Ok(())
# }
```

> [!NOTE]
> `select_for_update` must run inside a transaction — calling it against a bare
> pool `conn` returns `OrmError::SelectForUpdateOutsideTransaction`. Use the
> Postgres `db.transaction(|conn| ...)` form (or `transaction_conn`, which works
> on both backends). `nowait`, `skip_locked`, and `lock_of` are Postgres-only;
> SQLite rejects them with `UnsupportedOnDialect`.

---

## Soft deletes

Add `soft_delete` to a model to keep deleted rows in the table. Deleting sets
`deleted_at` to the current time; normal queries then skip those rows. The
model needs a nullable timestamp field with that name:

```rust,compile
# use djangors_orm::Model;
use chrono::{DateTime, Utc};
use djangors_macros::Model;
use djangors_orm::q;

#[derive(Model, Debug, Clone)]
#[djangors(app = "shop", table_name = "shop_product", soft_delete)]
pub struct Product {
    #[djangors(primary_key, auto)]
    pub id: i64,

    pub name: String,

    pub deleted_at: Option<DateTime<Utc>>,
}

# async fn run(db: &djangors_db::Database, product: Product) -> Result<(), Box<dyn std::error::Error>> {
// Sets deleted_at; the row stays in the table.
product.delete(db).await?;

// Hidden from count, all, first, get, exists and aggregates.
let _ = Product::objects().count(db).await?;
let _ = Product::objects().filter(q!(name = "Widget"))?.first(db).await?;

// Include deleted rows, or select only them.
let _ = Product::objects().with_deleted().count(db).await?;
let _ = Product::objects().only_deleted().all(db).await?;

// Clear deleted_at again.
product.restore(db).await?;

// Soft-delete again, then remove the row for real.
product.delete(db).await?;
product.force_delete(db).await?;

// Querysets work the same way: delete() soft-deletes every matching live row.
let _ = Product::objects().filter(q!(name = "Old"))?.delete(db).await?;

// with_deleted().delete() and force_delete() are real DELETE statements.
let _ = Product::objects().only_deleted().force_delete(db).await?;
# Ok(())
# }
```

The methods on a soft-delete model are:

- **`instance.delete(db)`**: sets `deleted_at` by primary key. Returns `OrmError::NotFound` if no live row matches, so deleting an already deleted row is an error. The delete signals still fire.
- **`instance.restore(db)`**: sets `deleted_at` back to `NULL`. Returns `OrmError::NotFound` if the row is not soft-deleted.
- **`instance.force_delete(db)`**: removes the row from the table.
- **`QuerySet::delete(db)`**: soft-deletes the matching live rows and returns how many it changed. Rows that are already deleted are not touched.
- **`QuerySet::force_delete(db)`**: removes the matching live rows from the table.
- **`with_deleted()`**: includes deleted rows in the queryset. Calling `delete(db)` on it is a real `DELETE`.
- **`only_deleted()`**: restricts the queryset to deleted rows. On a model without `soft_delete` it matches nothing.

- **`QuerySet::<T>::delete_by_pk(db, pk)`**: soft-deletes by primary key, so the admin, REST viewsets and generic delete views soft-delete too. **`force_delete_by_pk`** removes the row.

Soft-deleted rows are hidden everywhere rows are loaded: `EXISTS` / `NOT EXISTS`
subqueries, `select_related` (a deleted related row comes back as `None`),
`prefetch_related`, and `m2m_related`. Raw SQL is not filtered.

> [!NOTE]
> Unique constraints still apply to soft-deleted rows. A deleted row keeps its
> values, so inserting a new row with the same value in a `unique` column fails
> until the old row is removed with `force_delete`.

Models without `soft_delete` are unchanged: `delete` removes the row.

## Expressions & Macros

### `q!` Macro
Constructs lookup filter expressions:
```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let qs = Question::objects().filter(q!(question_text = "What is your name?"))?;
# Ok(())
# }
```

Every field inside `q!` accepts a lookup suffix typed on the ident itself —
`q!(field__lt = v)`, `q!(field__in = vec![..])`, etc. — it is not a separate
feature. The full set lives in the [lookup table](#lookup-suffixes) above; here
are the ones you reach for most day-to-day:

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let qs = Choice::objects()
    // Range: votes > 5 AND votes <= 100
    .filter(q!(votes__gt = 5i32))?
    .filter(q!(votes__lte = 100i32))?

    // Null check — the field must be nullable (Option<T> / Option<ForeignKey>):
    .filter(q!(choice_text__isnull = false))?

    // Set membership and negation:
    .filter(q!(id__in = vec![1i64, 2, 3]))?
    .filter(q!(votes__ne = 0i32))?

    // Case-insensitive substring:
    .filter(q!(choice_text__icontains = "rust"))?;
# let _ = qs;
# Ok(())
# }
```

## Combining filters: OR, AND, NOT

A `q!(...)` produces a value you can combine with `|` (OR), `&` (AND), and `!`
(NOT), which is how Django's `Q` objects spell the same thing. Successive
`.filter()` calls are still `AND`ed together.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// WHERE (votes = 0 OR votes > 100) AND NOT (choice_text = 'spam')
let qs = Choice::objects()
    .filter(q!(votes = 0i32) | q!(votes__gt = 100i32))?
    .filter(!q!(choice_text = "spam"))?;

// `.exclude()` is the same as filtering on a negation.
let qs = Choice::objects().exclude(q!(votes = 0i32))?;
# let _ = qs;
# Ok(())
# }
```

### Comparing two columns: `q_f!`

Where `q!` compares a column to a bound value, `q_f!` compares two columns on
the same row — `F()` on the filter side. The lookup suffix rides on the
left-hand field, as it does in Django.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q_f, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// WHERE "id" <> "votes"  — neither side is a bind parameter.
let qs = Choice::objects().filter(q_f!(id__ne votes))?;
# let _ = qs;
# Ok(())
# }
```

### Correlated Subqueries: `Exists`, `NotExists`, `OuterRef`, `q_outer!`

Use `Exists` or `NotExists` with `q_outer!` and `OuterRef` to construct correlated subqueries:

```rust,illustrative
# use polls::models::{Question, Choice};
# use djangors_orm::{q, q_outer, Exists, NotExists, OuterRef, Model};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// Questions with at least one choice with votes > 0:
let popular_questions = Question::objects()
    .filter(
        Exists::<Choice>::new()
            .filter(q_outer!(question = OuterRef("id")))?
            .filter(q!(votes__gt = 0i32))?,
    )?;

// Questions without choice with votes > 0:
let unpopular_questions = Question::objects()
    .filter(
        NotExists::<Choice>::new()
            .filter(q_outer!(question = OuterRef("id")))?
            .filter(q!(votes__gt = 0i32))?,
    )?;
# let _ = (popular_questions, unpopular_questions);
# Ok(())
# }
```

> [!NOTE]
> An `Exists` or `NotExists` subquery whose filters contain no `OuterRef` is an uncorrelated subquery. It evaluates once for the entire query and does not correlate rows between the subquery and outer query.

### Grouping: `annotate` and `values`

`.annotate()` is Django's `.values(...).annotate(...)`: it groups by the given
fields and computes aggregates per group, returning `GroupRow`s rather than
model instances.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{Model, aggregate::AggExpr};
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let rows = Choice::objects()
    .annotate(db, &["question"], vec![("total", AggExpr::Sum { field: "votes" })])
    .await?;

for row in &rows {
    let question_id = row.key("question");
    let total = row.get("total");
    println!("{question_id:?} => {total:?}");
}
# Ok(())
# }
```

Ordering is dropped for grouped queries, because ordering by a column that is
not in the `GROUP BY` is not valid SQL; sort the returned rows in Rust instead.

`.values()` and `.values_list()` select a projection instead of whole models,
which is useful when a table has columns you do not want to pay to fetch:

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::Model;
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let rows = Question::objects().values(db, &["id", "question_text"]).await?;
let ids = Question::objects().values_list(db, "id").await?;
# let _ = (rows, ids);
# Ok(())
# }
```

### `set!` Macro & `F` Expressions
Constructs update assignment lists, supporting `F` expressions for in-database atomic updates:
```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::{q, Model};
# async fn run_update(db: &djangors_db::Database, choice_id: i64) -> Result<(), Box<dyn std::error::Error>> {
use djangors_orm::{set, F};

// Increment votes by 1 atomically in SQL: UPDATE polls_choice SET votes = votes + 1 WHERE ...
Choice::objects()
    .filter(q!(id = choice_id))?
    .update(db, set!(votes = F("votes") + 1))
    .await?;
# Ok(())
# }
```

---

## Model Instance Persistence Methods

Models derived with `#[derive(Model)]` implement instance CRUD operations:
- **`model.save(db).await`**: Inserts a new row into the database, ignoring `auto` fields, and returns a fresh model instance with database-assigned primary key values.
- **`model.update(db).await`**: Updates all non-primary key fields of an existing row matching `model.id`. Returns `Err(OrmError::NotFound)` if no row matched.
- **`model.delete(db).await`**: Deletes the row matching `model.id`. Returns `Err(OrmError::NotFound)` if no row was deleted.

> [!IMPORTANT]
> `save` is **INSERT-only** — it always creates a new row. Calling `save()` on a
> row you already fetched or previously saved inserts a *duplicate*; use
> `update()` for anything that already exists. There is no automatic
> new-vs-persisted detection — track that yourself.

### Constructing models with foreign keys (`ForeignKey::new`)

A `ForeignKey<T>` field is not magically constructed — build it with
`ForeignKey::new(id)` exactly like any other field value. You generally save the
parent first to obtain its `id`, then insert the child.

```rust,compile
# use polls::models::{Question, Choice};
# use djangors_orm::ForeignKey;
# async fn run(db: &djangors_db::Database) -> Result<(), Box<dyn std::error::Error>> {
let question = Question {
    id: 0,
    question_text: "What is your name?".to_string(),
    pub_date: chrono::Utc::now(),
};

let question = question.save(db).await?; // INSERT; `question.id` now holds the DB-assigned pk.

let choice = Choice {
    id: 0,
    question: ForeignKey::<Question>::new(question.id), // the FK wrapper.
    choice_text: "Alice".to_string(),
    votes: 0,
};
let choice = choice.save(db).await?;
# let _ = choice;
# Ok(())
# }
```

Writes that don't need the returned row (bulk insert of many children, e.g.)
can skip `save` and use `QuerySet::<T>::insert_raw(db, values)` or
`QuerySet::<T>::bulk_create(db, &items)` instead; `bulk_create` inserts many
rows in one statement.

## Relationships & Many-to-Many

`djangors-orm` supports Foreign Key, One-to-One, and Many-to-Many relationships.

### Foreign Keys (`ForeignKey<T>`)

Foreign keys reference the primary key of another model:

```rust,illustrative
#[djangors(foreign_key(on_delete = "cascade", related_name = "choices"))]
pub question: ForeignKey<Question>,
```

To access the related ID directly, read `choice.question.id`. To load the related model instance lazily, call `.fetch(&db)`:

```rust,illustrative
if let Some(question) = choice.question.fetch(db).await? {
    println!("{}", question.question_text);
}
```

### One-to-One (`OneToOne<T>`)

A `OneToOne<T>` relationship is declared using the `OneToOne` wrapper. While it behaves similarly to a `ForeignKey`, migrations will automatically emit a `UNIQUE` constraint on the underlying column to ensure uniqueness:

```rust,illustrative
#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_user_profile")]
pub struct UserProfile {
    #[djangors(primary_key, auto)]
    pub id: i64,
    #[djangors(foreign_key(on_delete = "cascade", related_name = "profile"))]
    pub user: OneToOne<User>,
}
```

Like foreign keys, you can resolve the relationship lazily via `.fetch(db)`:

```rust,illustrative
let profile = UserProfile::objects().get(db).await?;
if let Some(user) = profile.user.fetch(db).await? {
    println!("User name: {}", user.name);
}
```

### Many-to-Many (`ManyToMany<T>`)

A `ManyToMany<T>` relation uses the `ManyToMany` wrapper as a zero-sized marker field. This field does not correspond to any column in the model's table. Instead, a dedicated through (junction) table named `{source_table_name}_{field_name}` is created automatically by migrations.

```rust,illustrative
#[derive(Model, Debug, Clone)]
#[djangors(app = "polls", table_name = "polls_school")]
pub struct School {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub name: String,
    #[djangors(many_to_many(related_name = "schools"))]
    pub students: ManyToMany<Student>,
}
```

You can interact with Many-to-Many relationships using the free helper functions `m2m_related`, `m2m_add`, and `m2m_remove`:

```rust,illustrative
use djangors_orm::{m2m_add, m2m_related, m2m_remove};

// Add relationship entries
m2m_add::<School, Student, _>(db, school.id, "students", student.id).await?;

// Query related instances
let students = m2m_related::<School, Student, _>(db, school.id, "students").await?;

// Remove relationship entries
m2m_remove::<School, Student, _>(db, school.id, "students", student.id).await?;
```

## Query Result Caching

`djangors-orm` provides an opt-in, in-process, process-local query cache to avoid hitting the database for duplicate queries. 

### Enabling and Querying the Cache

You can enable result caching on a `QuerySet` via `.cache(duration)` and fetch cached results via `.cached_all(db)`:

```rust,illustrative
use std::time::Duration;

// First call queries the database and caches the results
let questions = Question::objects()
    .filter(q!(pub_date__year = 2026))?
    .cache(Duration::from_secs(60))
    .cached_all(db)
    .await?;

// Subsequent calls with the exact same SQL and parameters
// will hit the process-local cache instead of the database.
let cached_questions = Question::objects()
    .filter(q!(pub_date__year = 2026))?
    .cache(Duration::from_secs(60))
    .cached_all(db)
    .await?;
```

### Invalidation

Write operations (`save()`, `update()`, and `delete()`) automatically invalidate all cached entries associated with the affected model's table. 

> [!IMPORTANT]
> The query cache is **process-local** and resides entirely in-memory. In multi-process deployments, writes to one process will not invalidate the caches of other processes, which can lead to stale data being served. Use with caution in distributed environments.

## Database Routing

For multi-database setups, `djangors-orm` provides metadata-level routing via `ModelDatabaseMap`. This allows you to map specific model struct names to distinct database instances:

```rust,illustrative
use djangors_orm::ModelDatabaseMap;

// Construct routing map
let routing_map = ModelDatabaseMap::new(
    default_db,
    vec![
        ("User", auth_db),
        ("Session", session_db),
    ],
);

// Retrieve the database configured for a specific model
let db = routing_map.db_for::<User>();
```

## Compile-Time Direct JSON Serialization (`write_json`)

Every `#[derive(Model)]` struct automatically generates a specialized, Fastify-style (`fast-json-stringify`) JSON writer on the `Model` trait:

- **`model.write_json(&mut buf)`**: Writes all non-`ManyToMany` model fields directly into a `Vec<u8>` buffer with pre-escaped static `"\"field_name\":"` byte literals in lexicographical key order (using `itoa` for integers and `ryu` for floats), avoiding intermediate `Vec<(&'static str, BindValue)>` and `serde_json::Map` allocations.
- **`model.write_json_filtered(&mut buf, |field_name| ...)`**: Same direct writer, filtered by a field-name predicate (used by `djangors-rest` when a `FieldSet` restricts readable fields).

```rust,compile
# use polls::models::Question;
# use djangors_orm::Model;
# use chrono::Utc;
# fn main() {
let q = Question {
    id: 1,
    question_text: "Fast JSON?".to_string(),
    pub_date: Utc::now(),
};

let mut buf = Vec::with_capacity(128);
q.write_json(&mut buf);
assert!(std::str::from_utf8(&buf).unwrap().contains("\"question_text\":\"Fast JSON?\""));
# }
```

## JSON Fields

Use `serde_json::Value` (or `Option<serde_json::Value>`) on a `#[derive(Model)]` field to store structured JSON (`FieldKind::Json` -> `JSONB` on PostgreSQL, `TEXT` on SQLite):

```rust,illustrative
use djangors::macros::Model;
use serde_json::Value;

#[derive(Model, Debug, Clone)]
#[djangors(app = "catalog", table_name = "catalog_product")]
pub struct Product {
    #[djangors(primary_key, auto)]
    pub id: i64,
    pub attributes: Value,
}
```
