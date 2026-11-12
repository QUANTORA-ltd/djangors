# Model History

`djangors-contrib-history` records every create, update, and delete of the models you choose,
the same idea as django-simple-history or PaperTrail. Each change is stored as a row with the
object's field values as JSON, so you can show who-changed-what-when for an object.

It is built on the model signals that `#[derive(Model)]` already generates, so your models need
no extra attributes.

## Setup

Create the history table once at startup. It is safe to call on every start.

```rust,compile
use djangors::db::Database;
use djangors::history::ensure_history_table;
use djangors::orm::OrmError;

async fn setup(db: &Database) -> Result<(), OrmError> {
    ensure_history_table(db).await
}
```

The table is `djangors_model_history`. It holds the model's table name, the primary key as text,
the action, the JSON data, and a timestamp. A single table serves every tracked model.

## Tracking a model

Call `track` for each model you want to record. Pass the database and the model's two generated
signals:

```rust,compile
use djangors::db::Database;
use djangors::history::{ensure_history_table, history_for, track};
use djangors::orm::OrmError;
use djangors_macros::Model;

#[derive(Model, Debug, Clone)]
#[djangors(app = "shop", table_name = "shop_product")]
struct Product {
    #[djangors(primary_key, auto)]
    id: i64,
    #[djangors(max_length = 200)]
    name: String,
    price_cents: i64,
}

async fn wire_up(db: Database) -> Result<(), OrmError> {
    ensure_history_table(&db).await?;
    track::<Product>(
        db.clone(),
        Product::post_save_signal(),
        Product::post_delete_signal(),
    );

    let mug = Product {
        id: 0,
        name: "Mug".to_string(),
        price_cents: 1200,
    }
    .save(&db)
    .await?;

    let mut repriced = mug.clone();
    repriced.price_cents = 1000;
    repriced.update(&db).await?;

    let entries = history_for::<Product>(&db, mug.id).await?;
    println!("{} changes recorded", entries.len());
    Ok(())
}
```

Call `track` once per model, during startup, before the first write. Calling it twice for the
same model records each change twice.

Each change is recorded with one of two actions:

- `"saved"`: a `save()` (create) or an `update()` on an instance.
- `"deleted"`: a `delete()` on an instance.

The entry holds the object's field values at that moment, so a `"deleted"` entry shows the
object as it was just before it was removed.

The history row is written inside the model signal, before `save`, `update`, or `delete` returns.
If the history insert fails, the error is logged with `tracing` and the model operation still
succeeds. A failed history write never blocks or rolls back a change.

## Reading history

`history_for` returns the entries for one object, oldest first:

```rust,illustrative
let entries = djangors::history::history_for::<Product>(&db, 42).await?;
for entry in entries {
    println!("{} at {}: {}", entry.action, entry.changed_at, entry.data);
}
```

Each `HistoryEntry` has:

- `action`: `"saved"` or `"deleted"`.
- `data`: a `serde_json::Value` object mapping each field name to its value at that moment.
- `changed_at`: the time the change was recorded, in UTC.

The key you pass is the primary key as text, so `42` and `"42"` find the same object.

## Encrypted fields

Fields marked [`#[djangors(encrypted)]`](encrypted-fields.md) are encrypted in the history
table too, with the same key, so a history row never holds their plaintext. `history_for`
decrypts them, so `data` shows the plain value. If encrypting fails (for example, the key is
not set), the entry is skipped and an error is logged; the plaintext is never written instead.

## What is not recorded

Only changes made through a model instance are recorded. `update()` and `delete()` called on a
whole `QuerySet` (for example `Product::objects().filter(...).delete()`) do not send per-row
signals, so bulk changes are **not** recorded. If you need history for a bulk change, load the
rows and call `save`, `update`, or `delete` on each one.

Raw SQL that writes to a tracked table also bypasses the signals.

## Storage and size

Every change stores a full copy of the object's fields, not a diff. For large rows or models
that change often, the table grows quickly. Plan a retention policy, for example a scheduled
`DELETE` of entries older than a set date, and index the table by `(table_name, object_pk)` as
`ensure_history_table` already does.
