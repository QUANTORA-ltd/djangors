# Schema Migrations

Djangors provides a robust, declarative database schema migration system inspired by Django's migration engine. It tracks changes to your Rust model definitions and generates database-agnostic SQL migrations that can be applied to both PostgreSQL and SQLite.

---

## The Workflow

The typical migration cycle involves two commands:

1. **`dj makemigrations`**: Inspects your current models, compares them with the previous schema state saved in `.schema_snapshot.json`, and generates a new SQL migration file.
2. **`dj migrate`**: Executes any pending migration files against the target database and records their names in the `djangors_migrations` history table.

---

## Migration Files and Markers

Migrations are written as plain SQL files under your project's migrations directory (usually `src/migrations/` or `migrations/`). 

Each file contains custom SQL comment markers to separate the forward (`up`) migration from the backward (`down`) rollback logic.

### Forward and Backward Actions
* **`-- up`**: Denotes the SQL statements to apply when running the migration forward.
* **`-- down`**: Denotes the SQL statements to run when reverting the migration.
* **`-- no-down`**: Explicitly marks a migration as non-reversible. If a rollback is attempted on a migration containing `-- no-down`, Djangors returns a `MigrationError::NonInvertible` error and aborts.

Example migration file (`0001_initial.sql`):
```sql
-- up
CREATE TABLE "polls_question" (
    "id" SERIAL PRIMARY KEY,
    "question_text" VARCHAR(200) NOT NULL,
    "pub_date" TIMESTAMPTZ NOT NULL
);

-- down
DROP TABLE "polls_question";
```

---

## Schema Snapshots

To detect schema changes without connecting to a live database, Djangors stores a JSON representation of your model structures in a file named `.schema_snapshot.json` inside your migrations directory. 

When you run `dj makemigrations`:
1. Djangors parses your compiled Rust model attributes (`#[derive(Model)]`).
2. It loads `.schema_snapshot.json` to see the last recorded schema state.
3. It performs a structural diff between the models and the snapshot.
4. Based on the diff, it generates the appropriate `Operation` steps and writes the SQL migration.
5. It updates `.schema_snapshot.json` with the new schema state.

---

## The `Operation` Enum

Internally, migrations are planned as a sequence of `Operation` steps. The `Operation` enum in `djangors-migrations` represents the supported DDL instructions.

```rust,illustrative
pub enum Operation {
    CreateTable {
        table_name: String,
        columns: Vec<ColumnDef>,
    },
    AddColumn {
        table_name: String,
        column: ColumnDef,
    },
    DropColumn {
        table_name: String,
        column_name: String,
    },
    AlterColumnType {
        table_name: String,
        column_name: String,
        new_sql_type: String,
    },
    RenameColumn {
        table_name: String,
        old_name: String,
        new_name: String,
    },
    DropTable {
        table_name: String,
    },
    CreateIndex {
        index_name: String,
        table_name: String,
        columns: Vec<String>,
    },
    DropIndex {
        index_name: String,
    },
}
```

### Reversibility

Djangors divides operations into **reversible** (invertible) and **non-reversible** categories. 

Reversible operations can automatically generate the correct SQL for the `-- down` section when running `dj makemigrations`:

| Operation | Reversible? | Reverse Operation |
| --- | --- | --- |
| `CreateTable` | **Yes** | `DropTable` |
| `AddColumn` | **Yes** | `DropColumn` |
| `RenameColumn` | **Yes** | `RenameColumn` (with old and new names swapped) |
| `DropColumn` | **No** | None (data is lost, cannot be automatically reconstructed) |
| `DropTable` | **No** | None (table columns and data are lost) |
| `AlterColumnType` | **No** | None (reverting requires knowledge of the original type) |
| `CreateIndex` | **Yes** | `DropIndex` |
| `DropIndex` | **No** | None (recreating the index requires knowing its table and columns) |

If a migration contains a non-reversible operation, Djangors inserts the `-- no-down` marker in the generated SQL file.

---

## Migration History Tracking

Djangors tracks which migrations have been applied using the `djangors_migrations` table inside your database.

The schema of this history table is:
* **`id`**: Auto-incrementing primary key.
* **`name`**: The migration filename without the `.sql` extension (e.g. `0001_initial`).

When running `dj migrate`, the system:
1. Ensures the `djangors_migrations` table exists.
2. Queries the table for all applied migration names.
3. Scans your migrations directory for `.sql` files.
4. Executes the SQL inside the `-- up` section of any files not recorded in `djangors_migrations`.
5. Inserts the names of newly applied migrations into `djangors_migrations` inside a transaction.

---

## Rolling Back Migrations

To undo the most recently applied migrations, use the rollback command or call `rollback_from_dir` programmatically:

```bash
# Roll back the single most recent migration
dj migrate --rollback 1
```

During a rollback, the system:
1. Queries the most recently applied migrations from the `djangors_migrations` table.
2. Reads their corresponding `.sql` files from disk.
3. Asserts that the files do not contain the `-- no-down` marker.
4. Executes the SQL inside the `-- down` section.
5. Deletes the migration records from the `djangors_migrations` table.

---

## Programmatic Migration APIs

Beyond the `dj` CLI, `djangors_migrations` exposes the same functionality as callable functions and
`Operation` methods, so you can wire migrations into your own startup or tooling.

### Database-level functions

| Function | Signature | Notes |
| --- | --- | --- |
| `migrate(&db)` | `async fn(&Database) -> Result<(), MigrationError>` | Applies `migrations/` dir if present, else the model-derived `0001_initial` plan |
| `migrate_from_dir(&db, path)` | `async fn(&Database, &Path) -> Result<(), MigrationError>` | Applies pending `.sql` files in filename order |
| `rollback_from_dir(&db, path, count)` | `async fn(&Database, &Path, u32) -> Result<(), MigrationError>` | Rolls back the most recent `count` applied files; errors on `-- no-down` |
| `get_applied_migrations(&db)` | `async fn(&Database) -> Result<Vec<String>, MigrationError>` | Names recorded in `djangors_migrations` |
| `record_migration(&db, name)` | `async fn(&Database, &str) -> Result<(), MigrationError>` | Records a name as applied without running SQL (the `--fake` behaviour) |

```rust,illustrative
use djangors_migrations::{get_applied_migrations, migrate, migrate_from_dir, record_migration, rollback_from_dir};

async fn manage(db: &djangors_db::Database) -> Result<(), djangors_migrations::MigrationError> {
    migrate(db).await?;                          // apply pending migrations

    let applied = get_applied_migrations(db).await?; // e.g. ["0001_initial"]
    if applied.is_empty() {
        record_migration(db, "0001_initial").await?; // record without running SQL
    }

    migrate_from_dir(db, std::path::Path::new("src/migrations")).await?;
    rollback_from_dir(db, std::path::Path::new("src/migrations"), 1).await?; // undo last 1
    Ok(())
}
```

### `Operation` methods

`Operation` is also usable directly as a plan of DDL steps:

| Method | Signature | Notes |
| --- | --- | --- |
| `to_sql(dialect)` | `fn(&self, Dialect) -> Result<String, MigrationError>` | Forward DDL SQL (`UnsupportedOnDialect` for SQLite-only gaps like `AlterColumnType`) |
| `reverse()` | `fn(&self) -> Option<Self>` | The mechanically safe inverse (`CreateTable`→`DropTable`, `AddColumn`→`DropColumn`, `RenameColumn` swapped); `None` when not invertible |
| `to_down_sql(dialect)` | `fn(&self, Dialect) -> Option<Result<String, MigrationError>>` | `reverse()` then `to_sql()` — the `-- down` section |

```rust,compile
# use djangors_db::Dialect;
# use djangors_migrations::Operation;
# fn op_demo() {
use djangors_migrations::ColumnDef;

let create = Operation::CreateTable {
    table_name: "polls_question".to_string(),
    columns: vec![ColumnDef {
        name: "id".to_string(),
        sql_type: "SERIAL".to_string(),
        nullable: false,
        primary_key: true,
        unique: false,
        default_sql: None,
        references: None,
    }],
    check_constraints: vec![],
};

let up_sql = create.to_sql(Dialect::Postgres).unwrap();   // CREATE TABLE ...
let down_sql = create.to_down_sql(Dialect::Postgres);     // Some("DROP TABLE ...")
assert!(matches!(create.reverse(), Some(Operation::DropTable { .. })));
# }
```

`build_create_all_plan(dialect)` (also `build_create_plan_from_snapshots`) produces the
`Vec<Operation>` plan for all registered models, topologically sorted by foreign-key dependency —
this is what `migrate` feeds into `to_sql` and executes transactionally.

---

## Detecting Model Changes with `detect_changes`

`detect_changes(previous, current, dialect)` diffs two sets of `ModelSnapshot` and returns the
incremental operations needed to bring the database from `previous` to `current`. For each model
present in `current` it emits:

* **New tables** — a model that exists in `current` but not in `previous` produces a `CreateTable`.
* **Added columns** — a field in `current` with no matching name in `previous` produces an
  `AddColumn`.
* **Dropped columns** — a field in `previous` with no matching name in `current` produces a
  `DropColumn`.
* **Changed column types** — a field present in both sets whose `kind` or `max_length` differs
  produces an `AlterColumnType`. Because `AlterColumnType` is unsupported on SQLite, diffing on a
  SQLite dialect surfaces an `UnsupportedOnDialect` error only when such a change is encountered.

The comparison keys off **field names**, not column identity. `detect_changes` does **not** detect
renames: renaming a column is seen as a *drop* of the old name plus an *add* of the new one, which
are emitted as separate `DropColumn` and `AddColumn` operations.

---

## Index Operations

Indexes declared through a model's struct-level `indexes` attribute are emitted into the generated
plan as `CreateIndex` operations, placed after the model's `CreateTable`. Each non-empty index
produces:

```rust,illustrative
Operation::CreateIndex {
    index_name: "idx_item_slug".to_string(), // from the index's name
    table_name: "app_item".to_string(),      // the model's table
    columns: vec!["slug".to_string()],       // the index's field names
}
```

rendered as `CREATE INDEX IF NOT EXISTS "idx_item_slug" ON "app_item" ("slug");`.

`DropIndex` carries only the index name and renders as
`DROP INDEX IF EXISTS "idx_item_slug";`. A `CreateIndex` reverses to `DropIndex`; a `DropIndex` has
no mechanical inverse.

---

## Many-to-Many Join Tables

A `ManyToMany` relation is not represented as a column on either model. Instead the plan emits a
dedicated join (through) table for it, named after the Django convention:

```
{source_table}_{field_name}
```

so a `students` `ManyToMany` field on a model whose table is `app_school` produces a through table
named `app_school_students`.

The through table carries exactly two non-nullable foreign-key columns, each `NOT NULL` and
referencing the target's primary key with `ON DELETE CASCADE`:

* **Source column**: `{source_struct_name_lowercase}_id` → references `{source_table}.{source_pk}`
* **Target column**: `{target_struct_name_lowercase}_id` → references `{target_table}.{target_pk}`

For the example above the plan emits:

```sql
CREATE TABLE IF NOT EXISTS "app_school_students" (
    "school_id" BIGINT NOT NULL REFERENCES "app_school"("id") ON DELETE CASCADE,
    "student_id" BIGINT NOT NULL REFERENCES "app_student"("id") ON DELETE CASCADE
);
```

The join table's own `id` column is not auto-added; the two FK columns are the only columns it
declares.

---

## OneToOne and the UNIQUE Constraint

A `OneToOne` relation is emitted as a plain foreign-key column like `ForeignKey`,
**plus** a UNIQUE constraint. The plan sets `unique: true` on the FK column specifically when the
relation kind is `OneToOne`, which is what distinguishes a one-to-one column from an ordinary
foreign key in the generated schema:

```sql
"profile_id" BIGINT UNIQUE REFERENCES "app_user"("id") ON DELETE CASCADE
```

---

## Migration Inspection & CLI Options

Djangors provides several commands and flags for inspecting and managing migration states:

### `dj sqlmigrate <app> <migration>`

Renders the SQL statements that a migration would execute without connecting to the database or modifying any state. The SQL output is dialect-correct based on `DATABASE_URL` (e.g. SQLite DDL when targeting SQLite, Postgres DDL when targeting PostgreSQL).

```bash
# Render SQL statements for migration 0001 in app polls
dj sqlmigrate polls 0001
```

### `dj showmigrations`

Lists every migration along with its applied state:

```bash
dj showmigrations
```

Output format:
```text
polls
 [X] 0001_initial
 [ ] 0002_add_field
```

* **`[X]`**: Migration is recorded in the `djangors_migrations` history table.
* **`[ ]`**: Migration exists on disk but has not been applied.
* **`[?]`**: Migration is recorded in the history table but missing from disk.

### `dj migrate --plan`

Prints the ordered list of migrations that would be applied without executing them:

```bash
dj migrate --plan
```

If the database is already up to date, it clearly reports that no migrations need to be applied.

### `dj migrate --fake`

Records migrations as applied in the `djangors_migrations` history table without executing their SQL DDL instructions:

```bash
dj migrate --fake
```

> [!WARNING]
> `--fake` can silently desynchronise the migration history table from the actual database schema. Use with caution when schema changes have been made out of band.

Note that `--fake`, `--plan`, and `--rollback` are mutually exclusive flags.
