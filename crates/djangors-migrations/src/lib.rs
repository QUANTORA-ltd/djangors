#![deny(missing_docs)]
//! Schema migration planning, execution, and SQL DDL generation for Djangors.
//!
//! # Database Connection Architecture: Enum Dispatch vs Generics
//! In Djangors, the database connection (`Conn`) and query row (`DbRow`) abstractions are designed using
//! runtime enum dispatch rather than trait generics (e.g., being generic over `<DB: sqlx::Database>`).
//! If generics were used, bounds like `DB: Database, for<'r> i64: Decode<'r, DB> + Type<DB>` would have to
//! propagate through every struct, trait method, derive macro, and downstream application crate.
//! Instead, `Conn` and `DbRow` wrap driver-specific types internally, resolving dialect differences (Postgres vs SQLite)
//! via runtime `match` statements in a single unified crate.
//!
//! Submodules:
//! - [`error`]: Migration error types.
//! - [`operation`]: DDL operation definitions and SQL generation logic.
//! - [`plan`]: Topological sorting and migration plan building.
//! - [`type_mapping`]: Mapping of ORM field types to SQL column types.
/// Migration error types.
pub mod error;
/// DDL operation definitions and SQL generation logic.
pub mod operation;
/// Topological sorting and migration plan building.
pub mod plan;
/// Mapping of ORM field types to SQL column types.
pub mod type_mapping;

pub use error::MigrationError;
pub use operation::{ColumnDef, ForeignKeyRef, Operation};
pub use plan::build_create_all_plan;
pub use plan::build_create_plan_from_snapshots;
pub use plan::detect_changes;

use djangors_db::BindValue;
use std::path::{Path, PathBuf};

/// Apply migration files in filename order. Files use `-- up` and `-- down` markers;
/// `-- no-down` explicitly records an unavailable reverse migration.
pub async fn migrate_from_dir(
    db: &djangors_db::Database,
    dir: &Path,
) -> Result<(), MigrationError> {
    ensure_history(db).await?;
    let dialect = db.dialect();
    let mut files = migration_files(dir)?;
    files.sort();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let query = format!(
            "SELECT 1 FROM djangors_migrations WHERE name = {}",
            dialect.placeholder(1)
        );
        let params = [BindValue::Text(name.clone())];
        let applied = db.conn().fetch_optional(&query, &params).await?.is_some();
        if applied {
            continue;
        }
        let content = std::fs::read_to_string(&path)?;
        let up = section(&content, "up");
        db.transaction_conn(|conn| {
            Box::pin(async move {
                for sql in up.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    let stmt = format!("{};", sql);
                    conn.execute(&stmt, &[]).await?;
                }
                let dialect = conn.dialect();
                let insert_sql = format!(
                    "INSERT INTO djangors_migrations (name) VALUES ({})",
                    dialect.placeholder(1)
                );
                conn.execute(&insert_sql, &[BindValue::Text(name)]).await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await
        .map_err(MigrationError::Database)?;
    }
    Ok(())
}

/// Roll back the most recent `count` applied migration files.
pub async fn rollback_from_dir(
    db: &djangors_db::Database,
    dir: &Path,
    count: u32,
) -> Result<(), MigrationError> {
    ensure_history(db).await?;
    // Scope strictly to migrations whose file still exists in `dir` - `djangors_migrations` is
    // a single shared tracking table, and without this filter "most recently applied" would be
    // read globally across any other migration set that happens to share the same table
    // (this matters for this crate's own concurrent test suite, and defensively protects any
    // real deployment where more than one migrations directory might share a database).
    let known_names: Vec<String> = migration_files(dir)?
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
        .collect();

    if known_names.is_empty() {
        return Ok(());
    }

    let dialect = db.dialect();
    let placeholders: Vec<String> = (1..=known_names.len())
        .map(|i| dialect.placeholder(i))
        .collect();
    let limit_placeholder = dialect.placeholder(known_names.len() + 1);
    let sql = format!(
        "SELECT name FROM djangors_migrations WHERE name IN ({}) ORDER BY id DESC LIMIT {}",
        placeholders.join(", "),
        limit_placeholder
    );

    let mut params: Vec<BindValue> = known_names
        .iter()
        .map(|n| BindValue::Text(n.clone()))
        .collect();
    params.push(BindValue::I64(count as i64));

    let rows = db.conn().fetch_all(&sql, &params).await?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(n) = row.try_string(0)? {
            names.push(n);
        }
    }

    let mut downs = Vec::new();
    for name in &names {
        let path = dir.join(format!("{name}.sql"));
        let text = std::fs::read_to_string(&path)
            .map_err(|_| MigrationError::NonInvertible { name: name.clone() })?;
        let down = section(&text, "down");
        if down.trim().is_empty() || text.contains("-- no-down") {
            return Err(MigrationError::NonInvertible { name: name.clone() });
        }
        downs.push((name.clone(), down));
    }
    for (name, down) in downs {
        db.transaction_conn(|conn| {
            Box::pin(async move {
                for sql in down.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                    let stmt = format!("{};", sql);
                    conn.execute(&stmt, &[]).await?;
                }
                let dialect = conn.dialect();
                let delete_sql = format!(
                    "DELETE FROM djangors_migrations WHERE name = {}",
                    dialect.placeholder(1)
                );
                conn.execute(&delete_sql, &[BindValue::Text(name)]).await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await
        .map_err(MigrationError::Database)?;
    }
    Ok(())
}

async fn ensure_history(db: &djangors_db::Database) -> Result<(), MigrationError> {
    let dialect = db.dialect();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_migrations (\
            id {}, \
            name TEXT UNIQUE NOT NULL, \
            applied_at {} NOT NULL DEFAULT {}\
        )",
        dialect.auto_pk_type(),
        dialect.timestamp_type(),
        dialect.current_timestamp()
    );
    db.conn().execute(&sql, &[]).await?;
    Ok(())
}

/// Checks that every registered model table and column exists in the database.
///
/// This check deliberately reports drift instead of applying schema changes
/// implicitly. Generate and commit a migration with `dj makemigrations`, then
/// apply it with `dj migrate` before starting the application.
pub async fn validate_model_schema(
    db: &djangors_db::Database,
    models: &[djangors_orm::ModelSnapshot],
) -> Result<(), MigrationError> {
    let dialect = db.dialect();
    for model in models {
        let mut conn = db.conn();
        let rows = match dialect {
            djangors_db::Dialect::Postgres => {
                conn
                    .fetch_all(
                        "SELECT column_name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1",
                        &[BindValue::Text(model.table_name.clone())],
                    )
                    .await?
            }
            djangors_db::Dialect::Sqlite => {
                let table = model.table_name.replace('"', "\"\"");
                conn
                    .fetch_all(&format!("PRAGMA table_info(\"{table}\")"), &[])
                    .await?
            }
        };
        let mut columns = std::collections::HashSet::new();
        for row in rows {
            let name = match dialect {
                djangors_db::Dialect::Postgres => row.try_string_by_name("column_name")?,
                djangors_db::Dialect::Sqlite => row.try_string_by_name("name")?,
            };
            if let Some(name) = name {
                columns.insert(name);
            }
        }
        let missing: Vec<String> = if columns.is_empty() {
            vec!["<table>".to_string()]
        } else {
            model
                .fields
                .iter()
                .filter(|field| !columns.contains(&field.column_name))
                .map(|field| field.column_name.clone())
                .collect()
        };
        if !missing.is_empty() {
            return Err(MigrationError::SchemaBehind {
                table: model.table_name.clone(),
                missing,
            });
        }
    }
    Ok(())
}

fn migration_files(dir: &Path) -> Result<Vec<PathBuf>, MigrationError> {
    Ok(std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|x| x == "sql")
                && p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .get(..4)
                    .and_then(|x| x.parse::<u32>().ok())
                    .is_some()
        })
        .collect())
}

fn section(text: &str, wanted: &str) -> String {
    let mut current = "";
    let mut start = 0;
    for (i, line) in text.lines().enumerate() {
        if line.trim() == format!("-- {wanted}") {
            current = wanted;
            start = text.lines().take(i + 1).map(|l| l.len() + 1).sum();
        } else if (line.trim() == "-- down" || line.trim() == "-- up") && current == wanted {
            return text[start..text.lines().take(i).map(|l| l.len() + 1).sum::<usize>()]
                .to_string();
        }
    }
    if current == wanted {
        text[start..].to_string()
    } else {
        String::new()
    }
}

/// Applies initial database schema migrations if not already applied.
pub async fn migrate(db: &djangors_db::Database) -> Result<(), MigrationError> {
    let models = djangors_orm::meta::registered_models_snapshot();
    if Path::new("migrations").is_dir() {
        migrate_from_dir(db, Path::new("migrations")).await?;
        return validate_model_schema(db, &models).await;
    }
    // 1. Ensure tracking table exists
    ensure_history(db).await?;

    let dialect = db.dialect();

    // 2. Check if 0001_initial is applied
    let query = format!(
        "SELECT 1 FROM djangors_migrations WHERE name = {}",
        dialect.placeholder(1)
    );
    let params = [BindValue::Text("0001_initial".to_string())];
    let row = db.conn().fetch_optional(&query, &params).await?;

    if row.is_some() {
        return validate_model_schema(db, &models).await;
    }

    // 3. Build plan and execute in transaction
    let plan = build_create_all_plan(dialect)?;
    let mut sqls = Vec::new();
    for op in &plan {
        sqls.push(op.to_sql(dialect)?);
    }

    db.transaction_conn(|conn| {
        let sqls = sqls.clone();
        Box::pin(async move {
            for sql in sqls {
                conn.execute(&sql, &[]).await?;
            }
            let dialect = conn.dialect();
            let insert_sql = format!(
                "INSERT INTO djangors_migrations (name) VALUES ({})",
                dialect.placeholder(1)
            );
            conn.execute(&insert_sql, &[BindValue::Text("0001_initial".to_string())])
                .await?;
            Ok::<(), djangors_db::DbError>(())
        })
    })
    .await
    .map_err(MigrationError::Database)?;

    validate_model_schema(db, &models).await
}

/// Returns the list of migration names currently recorded as applied in `djangors_migrations`.
pub async fn get_applied_migrations(
    db: &djangors_db::Database,
) -> Result<Vec<String>, MigrationError> {
    ensure_history(db).await?;
    let sql = "SELECT name FROM djangors_migrations ORDER BY id ASC";
    let rows = db.conn().fetch_all(sql, &[]).await?;
    let mut names = Vec::new();
    for row in rows {
        if let Some(n) = row.try_string(0)? {
            names.push(n);
        }
    }
    Ok(names)
}

/// Records a migration name as applied in `djangors_migrations` without executing its SQL.
pub async fn record_migration(
    db: &djangors_db::Database,
    name: &str,
) -> Result<(), MigrationError> {
    ensure_history(db).await?;
    let dialect = db.dialect();
    let insert_sql = format!(
        "INSERT INTO djangors_migrations (name) VALUES ({})",
        dialect.placeholder(1)
    );
    db.conn()
        .execute(&insert_sql, &[BindValue::Text(name.to_string())])
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "migrate_from_dir_tests.rs"]
mod migrate_from_dir_tests;
