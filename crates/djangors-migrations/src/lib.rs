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
pub use operation::{
    apply_data_migration, rollback_data_migration, ColumnDef, DataMigration, DataMigrationFn,
    ForeignKeyRef, MigrationFuture, Operation, RunRust, VectorIndexMethod, VectorOpClass,
};
pub use plan::build_create_all_plan;
pub use plan::build_create_plan_from_snapshots;
pub use plan::detect_changes;

use djangors_db::{BindValue, Dialect};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Options controlling how migrations from a directory are planned and applied.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct MigrateOptions {
    /// Record pending migrations in `djangors_migrations` without executing their SQL.
    pub fake: bool,
    /// If a pending migration only creates tables that already exist in the database,
    /// record it as applied instead of failing.
    pub fake_initial: bool,
    /// Return the planned migration steps without modifying the database.
    pub plan_only: bool,
}

/// A single step produced when planning or executing directory-based migrations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlanStep {
    /// Migration name (file stem without `.sql`).
    pub name: String,
    /// Action taken or planned (`"apply"`, `"fake"`, `"fake_initial"`, or `"plan"`).
    pub action: String,
}

/// Apply migration files in dependency/filename order. Files use `-- up` and `-- down` markers;
/// `-- no-down` explicitly records an unavailable reverse migration.
pub async fn migrate_from_dir(
    db: &djangors_db::Database,
    dir: &Path,
) -> Result<(), MigrationError> {
    migrate_from_dir_with_options(db, dir, MigrateOptions::default())
        .await
        .map(|_| ())
}

/// Plan or apply migration files from `dir` using the given [`MigrateOptions`].
///
/// Respects `-- dependencies: <dep1>, <dep2>` comment headers in `.sql` files via
/// topological sort, falling back to filename order for independent migrations.
pub async fn migrate_from_dir_with_options(
    db: &djangors_db::Database,
    dir: &Path,
    options: MigrateOptions,
) -> Result<Vec<MigrationPlanStep>, MigrationError> {
    let applied_set = if options.plan_only {
        if table_exists(db, "djangors_migrations").await? {
            load_applied_set(db).await?
        } else {
            HashSet::new()
        }
    } else {
        ensure_history(db).await?;
        load_applied_set(db).await?
    };

    let ordered = ordered_migration_files(dir)?;
    let mut steps = Vec::new();

    for (name, _path, content) in ordered {
        if applied_set.contains(&name) {
            continue;
        }
        let up = section(&content, "up");

        if options.plan_only {
            steps.push(MigrationPlanStep {
                name,
                action: "plan".to_string(),
            });
            continue;
        }

        if options.fake {
            insert_migration_record(db, &name).await?;
            steps.push(MigrationPlanStep {
                name,
                action: "fake".to_string(),
            });
            continue;
        }

        if options.fake_initial {
            if let Some(tables) = extract_only_create_table_names(&up) {
                let mut all_exist = !tables.is_empty();
                for tbl in &tables {
                    if !table_exists(db, tbl).await? {
                        all_exist = false;
                        break;
                    }
                }
                if all_exist {
                    insert_migration_record(db, &name).await?;
                    steps.push(MigrationPlanStep {
                        name,
                        action: "fake_initial".to_string(),
                    });
                    continue;
                }
            }
        }

        let migration_name = name.clone();
        db.transaction_conn(|conn| {
            Box::pin(async move {
                for sql in split_sql_statements(&up) {
                    let stmt = format!("{};", sql);
                    conn.execute(&stmt, &[]).await?;
                }
                let dialect = conn.dialect();
                let insert_sql = format!(
                    "INSERT INTO djangors_migrations (name) VALUES ({})",
                    dialect.placeholder(1)
                );
                conn.execute(&insert_sql, &[BindValue::Text(migration_name)])
                    .await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await
        .map_err(MigrationError::Database)?;

        steps.push(MigrationPlanStep {
            name,
            action: "apply".to_string(),
        });
    }

    Ok(steps)
}

/// Squashes a contiguous range of `.sql` migrations in `dir` (from `start_prefix` through
/// `end_prefix`, inclusive) into a single migration file named `squashed_name`.
///
/// The generated file includes a `-- replaces: ...` header listing all replaced migrations,
/// preserves any external `-- dependencies: ...` outside the squashed range, concatenates
/// the `-- up` sections in execution order, and emits reversed `-- down` sections (or
/// `-- no-down` if any migration in the range is non-invertible).
pub fn squash_migrations(
    dir: &Path,
    start_prefix: &str,
    end_prefix: &str,
    squashed_name: &str,
) -> Result<PathBuf, MigrationError> {
    let ordered = ordered_migration_files(dir)?;
    let start_idx = ordered
        .iter()
        .position(|(name, _, _)| name.starts_with(start_prefix))
        .ok_or_else(|| {
            MigrationError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("start migration prefix '{start_prefix}' not found"),
            ))
        })?;
    let end_idx = ordered
        .iter()
        .rposition(|(name, _, _)| name.starts_with(end_prefix))
        .ok_or_else(|| {
            MigrationError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("end migration prefix '{end_prefix}' not found"),
            ))
        })?;
    if start_idx > end_idx {
        return Err(MigrationError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("start prefix '{start_prefix}' appears after end prefix '{end_prefix}'"),
        )));
    }

    let slice = &ordered[start_idx..=end_idx];
    let replaced_names: Vec<String> = slice.iter().map(|(name, _, _)| name.clone()).collect();
    let replaced_set: HashSet<&str> = replaced_names.iter().map(String::as_str).collect();

    let mut external_deps: Vec<String> = Vec::new();
    let mut seen_deps: HashSet<String> = HashSet::new();
    let mut up_sections: Vec<String> = Vec::new();
    let mut down_sections: Vec<String> = Vec::new();
    let mut all_invertible = true;

    for (_, _, content) in slice {
        for dep in parse_dependencies(content) {
            if !replaced_set.contains(dep.as_str()) && seen_deps.insert(dep.clone()) {
                external_deps.push(dep);
            }
        }
        let up = section(content, "up").trim().to_string();
        if !up.is_empty() {
            up_sections.push(up);
        }
        let down = section(content, "down").trim().to_string();
        if down.is_empty() || content.contains("-- no-down") {
            all_invertible = false;
        } else {
            down_sections.push(down);
        }
    }

    let mut out = String::new();
    out.push_str(&format!("-- replaces: {}\n", replaced_names.join(", ")));
    if !external_deps.is_empty() {
        out.push_str(&format!("-- dependencies: {}\n", external_deps.join(", ")));
    }
    out.push_str("-- up\n");
    if !up_sections.is_empty() {
        out.push_str(&up_sections.join("\n\n"));
        out.push('\n');
    }
    out.push_str("-- down\n");
    if all_invertible && !down_sections.is_empty() {
        down_sections.reverse();
        out.push_str(&down_sections.join("\n\n"));
        out.push('\n');
    } else {
        out.push_str("-- no-down\n");
    }

    let filename = if squashed_name.ends_with(".sql") {
        squashed_name.to_string()
    } else {
        format!("{squashed_name}.sql")
    };
    let out_path = dir.join(filename);
    std::fs::write(&out_path, out)?;
    Ok(out_path)
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

pub(crate) async fn ensure_history(db: &djangors_db::Database) -> Result<(), MigrationError> {
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

async fn load_applied_set(db: &djangors_db::Database) -> Result<HashSet<String>, MigrationError> {
    let rows = db
        .conn()
        .fetch_all("SELECT name FROM djangors_migrations", &[])
        .await?;
    let mut set = HashSet::with_capacity(rows.len());
    for row in rows {
        if let Some(n) = row.try_string(0)? {
            set.insert(n);
        }
    }
    Ok(set)
}

async fn insert_migration_record(
    db: &djangors_db::Database,
    name: &str,
) -> Result<(), MigrationError> {
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

async fn table_exists(
    db: &djangors_db::Database,
    table_name: &str,
) -> Result<bool, MigrationError> {
    let sql = match db.dialect() {
        Dialect::Sqlite => "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
        Dialect::Postgres => {
            "SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1"
        }
        Dialect::MySql => {
            "SELECT 1 FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?"
        }
    };
    let row = db
        .conn()
        .fetch_optional(sql, &[BindValue::Text(table_name.to_string())])
        .await?;
    Ok(row.is_some())
}

fn parse_dependencies(content: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        let rest = trimmed
            .strip_prefix("-- dependencies:")
            .or_else(|| trimmed.strip_prefix("-- Dependencies:"));
        if let Some(list) = rest {
            for item in list.split(',') {
                let dep = item.trim();
                if !dep.is_empty() {
                    deps.push(dep.to_string());
                }
            }
        }
    }
    deps
}

fn ordered_migration_files(dir: &Path) -> Result<Vec<(String, PathBuf, String)>, MigrationError> {
    let files = migration_files(dir)?;
    let mut by_name: BTreeMap<String, (PathBuf, String)> = BTreeMap::new();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let content = std::fs::read_to_string(&path)?;
        by_name.insert(name, (path, content));
    }

    let mut in_degree: HashMap<String, usize> = HashMap::with_capacity(by_name.len());
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();
    for name in by_name.keys() {
        in_degree.insert(name.clone(), 0);
    }

    for (name, (_, content)) in &by_name {
        let mut seen = HashSet::new();
        for dep in parse_dependencies(content) {
            if dep != *name && by_name.contains_key(&dep) && seen.insert(dep.clone()) {
                *in_degree.get_mut(name).unwrap() += 1;
                dependents.entry(dep).or_default().push(name.clone());
            }
        }
    }

    let mut ready: BTreeSet<String> = in_degree
        .iter()
        .filter_map(|(name, &deg)| if deg == 0 { Some(name.clone()) } else { None })
        .collect();

    let total = by_name.len();
    let mut ordered = Vec::with_capacity(total);
    while let Some(name) = ready.pop_first() {
        if let Some(deps_on_this) = dependents.get(&name) {
            for child in deps_on_this {
                let deg = in_degree.get_mut(child).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    ready.insert(child.clone());
                }
            }
        }
        let (path, content) = by_name.remove(&name).unwrap();
        ordered.push((name, path, content));
    }

    if ordered.len() < total {
        let remaining: Vec<String> = by_name.into_keys().collect();
        return Err(MigrationError::CyclicDependency { models: remaining });
    }

    Ok(ordered)
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(|chunk| {
            chunk
                .lines()
                .filter(|line| !line.trim().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn extract_only_create_table_names(up_sql: &str) -> Option<Vec<String>> {
    let stmts = split_sql_statements(up_sql);
    if stmts.is_empty() {
        return None;
    }
    let mut tables = Vec::with_capacity(stmts.len());
    for stmt in stmts {
        let mut tokens = stmt.split_whitespace();
        let first = tokens.next()?.to_ascii_uppercase();
        let second = tokens.next()?.to_ascii_uppercase();
        if first != "CREATE" || second != "TABLE" {
            return None;
        }
        let mut next = tokens.next()?;
        if next.eq_ignore_ascii_case("IF") {
            let not_kw = tokens.next()?;
            let exists_kw = tokens.next()?;
            if !not_kw.eq_ignore_ascii_case("NOT") || !exists_kw.eq_ignore_ascii_case("EXISTS") {
                return None;
            }
            next = tokens.next()?;
        }
        let raw_ident = next.split('(').next().unwrap_or(next);
        let unqualified = raw_ident.rsplit('.').next().unwrap_or(raw_ident);
        let clean = unqualified
            .trim_matches(|c| matches!(c, '"' | '`' | '[' | ']'))
            .to_string();
        if clean.is_empty() {
            return None;
        }
        tables.push(clean);
    }
    Some(tables)
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
    let marker = format!("-- {wanted}");
    let mut active = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == marker {
            active = true;
            continue;
        }
        if active && (trimmed == "-- down" || trimmed == "-- up") {
            break;
        }
        if active {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Applies initial database schema migrations if not already applied.
pub async fn migrate(db: &djangors_db::Database) -> Result<(), MigrationError> {
    if Path::new("migrations").is_dir() {
        return migrate_from_dir(db, Path::new("migrations")).await;
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
        return Ok(());
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

    Ok(())
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
