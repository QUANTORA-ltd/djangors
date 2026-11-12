# Code Generation: Resources, `inspectdb`, and TypeScript Clients

Three `dj` commands write code for you, so you don't type the same boilerplate for
every model:

| Command | Writes | Closest equivalent elsewhere |
| :--- | :--- | :--- |
| `dj generate resource` | Model, serializer, REST views, admin, tests, and a SQL migration | `rails generate scaffold`, `php artisan make:model -a` |
| `dj inspectdb` | `#[derive(Model)]` structs from an existing database | Django's `manage.py inspectdb` |
| `dj openapi --typescript` | A typed TypeScript client for your REST API | `openapi-typescript`, tRPC-style typed clients |

All three print to the terminal, or to a file with `--output`, and every file they
write is ordinary Rust or TypeScript that you own and can edit.

## `dj generate resource`

Run it from your project root, giving a PascalCase model name and `name:type` field
specs:

```bash
dj generate resource Post title:string body:text published:bool
```

This creates a module under `src/posts/` plus a numbered migration:

```text
src/posts/mod.rs
src/posts/models.rs        # #[derive(Model)] struct Post
src/posts/serializers.rs   # REST serializer
src/posts/views.rs         # ViewSet + register_routes(router)
src/posts/admin.rs         # register_admin(&site)
src/posts/tests.rs         # a working test for the generated model
migrations/0001_create_posts_post.sql
```

The module name defaults to the snake_case plural of the model (`BlogComment` becomes
`blog_comments`). Pass `--app <name>` to choose it yourself.

### Field types

| Spec | Rust type | PostgreSQL column | SQLite column |
| :--- | :--- | :--- | :--- |
| `string` (`varchar`, `char`) | `String` with `max_length = 255` | `VARCHAR(255)` | `TEXT` |
| `text` | `String` | `TEXT` | `TEXT` |
| `int` (`i32`, `integer`) | `i32` | `INTEGER` | `INTEGER` |
| `bigint` (`i64`) | `i64` | `BIGINT` | `INTEGER` |
| `float` (`f64`) | `f64` | `DOUBLE PRECISION` | `REAL` |
| `bool` (`boolean`) | `bool` | `BOOLEAN DEFAULT FALSE` | `INTEGER DEFAULT 0` |
| `datetime` (`timestamp`) | `chrono::DateTime<chrono::Utc>` | `TIMESTAMPTZ` | `TEXT` |
| `uuid` | `uuid::Uuid` | `UUID` | `TEXT` |
| `decimal` | `rust_decimal::Decimal` | `NUMERIC(19, 4)` | `NUMERIC` |

Every column is `NOT NULL`. The migration uses SQLite column types when `DATABASE_URL`
is a SQLite URL and PostgreSQL types otherwise, so set `DATABASE_URL` before generating.

### Wiring the resource in

The command prints these steps when it finishes:

1. Add `mod posts;` to `src/main.rs`.
2. Make sure `Cargo.toml` depends on `djangors-macros`, `djangors-orm`, `djangors-rest`,
   `djangors-admin`, `serde` (with the `derive` feature), and `serde_json`, plus
   `chrono`, `uuid`, or `rust_decimal` if your fields use them. Depending on the
   `djangors` facade crate covers the framework crates.
3. Mount the routes: `let router = posts::views::register_routes(router);`
4. Register the admin: `posts::admin::register_admin(&site);`
5. Run `dj migrate`.

After that, `cargo test` runs the generated test against your database.

## `dj inspectdb`

Point `dj inspectdb` at an existing PostgreSQL or SQLite database to get model structs
for its tables:

```bash
dj inspectdb --database-url postgres://localhost/legacy --app legacy --output src/legacy/models.rs
```

| Flag | Default | Meaning |
| :--- | :--- | :--- |
| `--database-url` | `DATABASE_URL` | Database to read |
| `--app` | `app` | Value for each struct's `#[djangors(app = "...")]` |
| `--output`, `-o` | stdout | File to write |

The output keeps each table name (`table_name = "..."`), marks the primary key, turns
nullable columns into `Option<T>`, and carries `VARCHAR(n)` lengths over as
`max_length`. A column whose name isn't a valid Rust identifier, such as `type`,
`self`, or `first name`, gets a valid field name plus a `column = "..."` attribute
that maps it back to the real column.

Djangors has no field type yet for some columns: JSON, `bytea`, `inet`, arrays,
intervals, and geometric types. `dj inspectdb` generates those fields as `String` so the
file still compiles, and prints a warning to stderr for each one. Reads and writes of
those fields fail until you change them, so handle every warning before using the model:

```text
[dj inspectdb] warning: legacy_event.payload: column type `jsonb` has no Djangors field type; generated as String (reads and writes will fail until you change it)
```

Read the generated models before you use them: like Django's version, `inspectdb` is a
starting point, not a guarantee.

## `dj openapi` and the TypeScript client

`dj openapi` reads the models compiled into your project and writes an OpenAPI
specification for their REST endpoints:

```bash
dj openapi --output openapi.json
```

Add `--typescript` to get a typed client with no dependencies instead:

```bash
dj openapi --typescript --output frontend/src/api.ts
```

The generated file contains:

- An interface per model (`Post`), plus `PostCreateInput` and `PostUpdateInput`
  (all fields optional, for `PATCH`).
- `PaginatedList<T>` for list responses.
- `ProblemDetails` and a `ProblemDetailsError` class: every failed request throws a
  `ProblemDetailsError`, whether the server sent
  [RFC 9457 problem details](http-and-middleware.md#standard-problem-details-rfc-9457)
  or the default Djangors JSON error.
- A `DjangorsApiClient` class with `list`, `retrieve`, `create`, `update`,
  `partialUpdate`, and `destroy` methods per model.

```ts
import { DjangorsApiClient, ProblemDetailsError } from "./api";

const api = new DjangorsApiClient({
  baseUrl: "https://example.com",
  headers: { Authorization: `Token ${token}` },
});

const page = await api.listPost({ published: true });
try {
  await api.createPost({ title: "Hello", body: "First post", published: false });
} catch (err) {
  if (err instanceof ProblemDetailsError) console.error(err.problem.errors);
}
```

`ClientOptions.fetch` lets you pass your own `fetch` implementation, for example in
tests or older runtimes.

**Check the paths.** By default, the generator assumes each model is served at
`/api/<plural>/`, the layout `dj generate resource` produces. Set `--api-prefix <PREFIX>`
to change the prefix and repeat `--model <NAME>` to include only matching Rust struct names.
If you mount a ViewSet
somewhere else, or don't expose a model over REST at all, edit the generated paths or
delete those methods. Regenerate the client whenever your models change; the file
starts with a "do not edit" header for that reason.
