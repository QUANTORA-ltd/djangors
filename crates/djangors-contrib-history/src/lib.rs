#![deny(missing_docs)]
//! Model history for Djangors: records every create, update, and delete of chosen models, in the
//! spirit of django-simple-history and PaperTrail.
//!
//! History is built on the model signals that `#[derive(Model)]` already generates, so no macro
//! changes are needed. Create the shared table once with [`ensure_history_table`], then connect
//! each model you want to record with [`track`]. Read a row's timeline back with [`history_for`].
//!
//! Only per-row writes made through the model (`save`, `update`, `delete` on an instance) are
//! recorded. Bulk operations on a whole `QuerySet` do not send per-row signals and are not
//! recorded.

use chrono::Utc;
use djangors_db::{BindValue, Database, DbRow};
use djangors_orm::signals::{ModelSignal, ModelSignalPayload};
use djangors_orm::{Model, OrmError, Value};

/// One recorded change to a tracked object.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// What happened: `"saved"` for a create or update, `"deleted"` for a delete.
    pub action: String,
    /// The object's field values at that moment, as a JSON object (`field name -> value`).
    pub data: serde_json::Value,
    /// When the change was recorded (UTC).
    pub changed_at: chrono::DateTime<Utc>,
}

/// Creates the `djangors_model_history` table and its `(table_name, object_pk)` index if they
/// do not exist.
///
/// Call this once at startup, before any tracked model is saved or deleted.
pub async fn ensure_history_table(db: &Database) -> Result<(), OrmError> {
    let dialect = db.dialect();
    let table_sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_model_history (\
         id {}, \
         table_name TEXT NOT NULL, \
         object_pk TEXT NOT NULL, \
         action TEXT NOT NULL, \
         data TEXT NOT NULL, \
         changed_at {} NOT NULL)",
        dialect.auto_pk_type(),
        dialect.timestamp_type(),
    );
    let index_sql = "CREATE INDEX IF NOT EXISTS djangors_model_history_object_idx \
         ON djangors_model_history (table_name, object_pk)";
    let mut conn = db.conn();
    conn.execute(&table_sql, &[]).await?;
    conn.execute(index_sql, &[]).await?;
    Ok(())
}

/// Starts recording every save and delete of model `M` into `djangors_model_history`.
///
/// Pass the model's generated signals, which the derive provides on every model:
///
/// ```rust,ignore
/// djangors::history::track::<Post>(
///     db.clone(),
///     Post::post_save_signal(),
///     Post::post_delete_signal(),
/// );
/// ```
///
/// Each `save` or `update` records a `"saved"` entry, and each `delete` records a `"deleted"`
/// entry, holding the object's field values as JSON. The write happens inside the signal, so it
/// completes before the model call that triggered it returns. A failed insert is logged with
/// `tracing::error!` and never panics or fails the model operation.
///
/// Call [`ensure_history_table`] before using this. Calling `track` again for the same model
/// records each change twice, so connect each model once.
pub fn track<M: Model + 'static>(
    db: Database,
    post_save: &'static ModelSignal<ModelSignalPayload>,
    post_delete: &'static ModelSignal<ModelSignalPayload>,
) {
    let meta = M::meta();
    let table_name = meta.table_name;
    let pk_name = meta
        .fields
        .iter()
        .find(|field| field.primary_key)
        .map(|field| field.name);
    let encrypted = M::encrypted_fields();

    let save_db = db.clone();
    post_save.connect(move |payload| {
        let db = save_db.clone();
        async move {
            record(&db, table_name, pk_name, encrypted, "saved", &payload).await;
        }
    });

    post_delete.connect(move |payload| {
        let db = db.clone();
        async move {
            record(&db, table_name, pk_name, encrypted, "deleted", &payload).await;
        }
    });
}

/// Returns the recorded history of one object of model `M`, oldest first.
///
/// `pk` is the object's primary key as text (for example `post.id` for an `i64` key). Returns an
/// empty list when nothing was recorded for that key.
pub async fn history_for<M: Model>(
    db: &Database,
    pk: impl ToString,
) -> Result<Vec<HistoryEntry>, OrmError> {
    let table_name = M::meta().table_name;
    let dialect = db.dialect();
    let sql = format!(
        "SELECT action, data, changed_at FROM djangors_model_history \
         WHERE table_name = {} AND object_pk = {} ORDER BY id ASC",
        dialect.placeholder(1),
        dialect.placeholder(2),
    );
    let params = [
        BindValue::Text(table_name.to_string()),
        BindValue::Text(pk.to_string()),
    ];
    let mut conn = db.conn();
    let rows = conn.fetch_all(&sql, &params).await?;
    let mut entries = rows
        .iter()
        .map(entry_from_row)
        .collect::<Result<Vec<_>, _>>()?;
    // `#[djangors(encrypted)]` fields are stored encrypted; return them decrypted, as a
    // loaded model would.
    let encrypted = M::encrypted_fields();
    for entry in &mut entries {
        if let serde_json::Value::Object(map) = &mut entry.data {
            for field in encrypted {
                if let Some(serde_json::Value::String(stored)) = map.get_mut(*field) {
                    *stored = djangors_orm::encryption::decrypt_str(stored)?;
                }
            }
        }
    }
    Ok(entries)
}

async fn record(
    db: &Database,
    table_name: &'static str,
    pk_name: Option<&'static str>,
    encrypted: &'static [&'static str],
    action: &'static str,
    payload: &ModelSignalPayload,
) {
    let object_pk = pk_name
        .and_then(|name| payload.iter().find(|(field, _)| *field == name))
        .and_then(|(_, value)| key_text(value));
    let Some(object_pk) = object_pk else {
        tracing::error!(
            table = table_name,
            action,
            "model history entry skipped: primary key missing from signal payload"
        );
        return;
    };

    let data = match payload_to_object(payload, encrypted) {
        Ok(object) => serde_json::Value::Object(object).to_string(),
        Err(err) => {
            // Never fall back to storing an encrypted field's plaintext.
            tracing::error!(
                table = table_name,
                action,
                error = %err,
                "model history entry skipped: encrypting a field failed"
            );
            return;
        }
    };
    let dialect = db.dialect();
    let placeholders: Vec<String> = (1..=5).map(|i| dialect.placeholder(i)).collect();
    let sql = format!(
        "INSERT INTO djangors_model_history (table_name, object_pk, action, data, changed_at) \
         VALUES ({})",
        placeholders.join(", ")
    );
    let params = [
        BindValue::Text(table_name.to_string()),
        BindValue::Text(object_pk),
        BindValue::Text(action.to_string()),
        BindValue::Text(data),
        BindValue::DateTime(Utc::now()),
    ];
    let mut conn = db.conn();
    if let Err(err) = conn.execute(&sql, &params).await {
        tracing::error!(
            table = table_name,
            action,
            error = %err,
            "model history insert failed"
        );
    }
}

fn entry_from_row(row: &DbRow) -> Result<HistoryEntry, OrmError> {
    let action = row
        .try_string(0)?
        .ok_or_else(|| OrmError::InvalidQuery("model history row has no action".to_string()))?;
    let data_text = row
        .try_string(1)?
        .ok_or_else(|| OrmError::InvalidQuery("model history row has no data".to_string()))?;
    let data = serde_json::from_str(&data_text).map_err(|err| {
        OrmError::InvalidQuery(format!("model history row has invalid data: {err}"))
    })?;
    let changed_at = row
        .try_datetime(2)?
        .ok_or_else(|| OrmError::InvalidQuery("model history row has no changed_at".to_string()))?;
    Ok(HistoryEntry {
        action,
        data,
        changed_at,
    })
}

/// Converts a signal payload to a JSON object, encrypting the `encrypted` fields so the
/// history table never holds their plaintext.
fn payload_to_object(
    payload: &ModelSignalPayload,
    encrypted: &[&str],
) -> Result<serde_json::Map<String, serde_json::Value>, OrmError> {
    payload
        .iter()
        .map(|(name, value)| {
            let json = match value {
                Value::Text(text) if encrypted.contains(name) => {
                    serde_json::Value::String(djangors_orm::encryption::encrypt_str(text)?)
                }
                other => value_to_json(other),
            };
            Ok(((*name).to_string(), json))
        })
        .collect()
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::I64(v) => serde_json::Value::from(*v),
        Value::F64(v) => serde_json::Number::from_f64(*v)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::Text(v) => serde_json::Value::String(v.clone()),
        Value::Bool(v) => serde_json::Value::Bool(*v),
        Value::DateTime(v) => serde_json::Value::String(v.to_rfc3339()),
        Value::Null => serde_json::Value::Null,
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
    }
}

/// The primary key as the text stored in `object_pk`. Returns `None` for values that cannot
/// identify a row (NULL or a list).
fn key_text(value: &Value) -> Option<String> {
    match value {
        Value::Text(text) => Some(text.clone()),
        Value::I64(v) => Some(v.to_string()),
        Value::F64(v) => Some(v.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        Value::DateTime(v) => Some(v.to_rfc3339()),
        Value::Null | Value::List(_) => None,
    }
}

#[cfg(test)]
mod tests;
