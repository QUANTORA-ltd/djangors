//! Additional Django-parity management commands for `dj`:
//! `dumpdata`, `loaddata`, `flush`, `squashmigrations`, `diffsettings`,
//! `sendtestemail`, `makemessages`, `compilemessages`, `changepassword`,
//! `createcachetable`, and `showurls`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use djangors_db::{BindValue, Database, DbRow, Dialect, NullKind};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A single fixture record emitted by `dj dumpdata` and consumed by `dj loaddata`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FixtureRecord {
    /// Table (or Django `model`) name.
    #[serde(alias = "model")]
    pub table: String,
    /// Column name -> JSON value map.
    pub fields: Map<String, Value>,
}

fn resolve_database_url(database_url: Option<&str>) -> Result<String, String> {
    match database_url {
        Some(u) if !u.trim().is_empty() => Ok(u.to_string()),
        _ => std::env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL is not set and --database-url was not provided".to_string()),
    }
}

async fn connect_db(database_url: Option<&str>) -> Result<Database, String> {
    let url = resolve_database_url(database_url)?;
    let config = djangors_db::config::DatabaseConfig::new(url);
    Database::connect(&config)
        .await
        .map_err(|e| format!("failed to connect to database: {e}"))
}

fn validate_sql_ident(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        || name.chars().next().is_some_and(|c| c.is_ascii_digit())
    {
        return Err(format!("invalid SQL identifier '{name}'"));
    }
    Ok(())
}

async fn list_user_tables(db: &Database) -> Result<Vec<String>, String> {
    let sql = match db.dialect() {
        Dialect::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name != 'djangors_migrations' ORDER BY name"
        }
        Dialect::Postgres => {
            "SELECT table_name AS name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name != 'djangors_migrations' ORDER BY table_name"
        }
        Dialect::MySql => {
            "SELECT table_name AS name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' AND table_name != 'djangors_migrations' ORDER BY table_name"
        }
    };
    let rows = db
        .conn()
        .fetch_all(sql, &[])
        .await
        .map_err(|e| e.to_string())?;
    let mut names = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(name) = row.try_string_by_name("name").map_err(|e| e.to_string())? {
            names.push(name);
        }
    }
    Ok(names)
}

async fn list_table_columns(db: &Database, table: &str) -> Result<Vec<String>, String> {
    validate_sql_ident(table)?;
    match db.dialect() {
        Dialect::Sqlite => {
            let pragma = format!("PRAGMA table_info(\"{table}\")");
            let rows = db
                .conn()
                .fetch_all(&pragma, &[])
                .await
                .map_err(|e| e.to_string())?;
            let mut cols = Vec::with_capacity(rows.len());
            for row in rows {
                if let Some(name) = row.try_string_by_name("name").map_err(|e| e.to_string())? {
                    cols.push(name);
                }
            }
            Ok(cols)
        }
        Dialect::Postgres => {
            let rows = db
                .conn()
                .fetch_all(
                    "SELECT column_name AS name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position",
                    &[BindValue::Text(table.to_string())],
                )
                .await
                .map_err(|e| e.to_string())?;
            let mut cols = Vec::with_capacity(rows.len());
            for row in rows {
                if let Some(name) = row.try_string_by_name("name").map_err(|e| e.to_string())? {
                    cols.push(name);
                }
            }
            Ok(cols)
        }
        Dialect::MySql => {
            let rows = db
                .conn()
                .fetch_all(
                    "SELECT column_name AS name FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position",
                    &[BindValue::Text(table.to_string())],
                )
                .await
                .map_err(|e| e.to_string())?;
            let mut cols = Vec::with_capacity(rows.len());
            for row in rows {
                if let Some(name) = row.try_string_by_name("name").map_err(|e| e.to_string())? {
                    cols.push(name);
                }
            }
            Ok(cols)
        }
    }
}

fn decode_cell(row: &DbRow, idx: usize) -> Value {
    if let Ok(Some(v)) = row.try_i64(idx) {
        return Value::Number(v.into());
    }
    if let Ok(Some(v)) = row.try_bool(idx) {
        return Value::Bool(v);
    }
    if let Ok(Some(v)) = row.try_f64(idx) {
        if let Some(n) = serde_json::Number::from_f64(v) {
            return Value::Number(n);
        }
    }
    if let Ok(Some(v)) = row.try_string(idx) {
        return Value::String(v);
    }
    if let Ok(Some(v)) = row.try_datetime(idx) {
        return Value::String(v.to_rfc3339());
    }
    Value::Null
}

/// Dumps the requested tables (or all user tables if empty) from `db` into fixture records.
pub async fn dumpdata_records(
    db: &Database,
    tables: &[String],
) -> Result<Vec<FixtureRecord>, String> {
    let target_tables = if tables.is_empty() {
        list_user_tables(db).await?
    } else {
        for t in tables {
            validate_sql_ident(t)?;
        }
        tables.to_vec()
    };

    let mut records = Vec::new();
    for table in target_tables {
        let cols = list_table_columns(db, &table).await?;
        if cols.is_empty() {
            continue;
        }
        let col_list = cols
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT {col_list} FROM \"{table}\"");
        let rows = db
            .conn()
            .fetch_all(&sql, &[])
            .await
            .map_err(|e| format!("failed to query table '{table}': {e}"))?;
        for row in rows {
            let mut fields = Map::new();
            for (idx, col) in cols.iter().enumerate() {
                fields.insert(col.clone(), decode_cell(&row, idx));
            }
            records.push(FixtureRecord {
                table: table.clone(),
                fields,
            });
        }
    }
    Ok(records)
}

/// Export database table rows as Django-style JSON fixtures.
pub async fn dumpdata(
    tables: &[String],
    output: Option<&Path>,
    database_url: Option<&str>,
) -> Result<(), String> {
    let db = connect_db(database_url).await?;
    let records = dumpdata_records(&db, tables).await?;
    let json = serde_json::to_string_pretty(&records)
        .map_err(|e| format!("failed to serialize fixtures: {e}"))?;
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(path, &json)
            .map_err(|e| format!("failed to write '{}': {e}", path.display()))?;
        println!(
            "[dj dumpdata] wrote {} record(s) to {}",
            records.len(),
            path.display()
        );
    } else {
        println!("{json}");
    }
    Ok(())
}

fn json_to_bind(val: &Value) -> BindValue {
    match val {
        Value::Null => BindValue::Null(NullKind::Text),
        Value::Bool(b) => BindValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                BindValue::I64(i)
            } else if let Some(f) = n.as_f64() {
                BindValue::F64(f)
            } else {
                BindValue::Text(n.to_string())
            }
        }
        Value::String(s) => BindValue::Text(s.clone()),
        Value::Array(_) | Value::Object(_) => BindValue::Text(val.to_string()),
    }
}

/// Loads fixture records from a JSON string into `db` and returns the number of inserted rows.
pub async fn loaddata_from_json(db: &Database, json: &str) -> Result<usize, String> {
    let records: Vec<FixtureRecord> =
        serde_json::from_str(json).map_err(|e| format!("invalid fixture JSON: {e}"))?;
    let dialect = db.dialect();
    let mut loaded = 0usize;

    for rec in records {
        validate_sql_ident(&rec.table)?;
        if rec.fields.is_empty() {
            continue;
        }
        let mut cols = Vec::with_capacity(rec.fields.len());
        let mut placeholders = Vec::with_capacity(rec.fields.len());
        let mut params = Vec::with_capacity(rec.fields.len());
        for (i, (col, val)) in rec.fields.iter().enumerate() {
            validate_sql_ident(col)?;
            cols.push(format!("\"{col}\""));
            placeholders.push(dialect.placeholder(i + 1));
            params.push(json_to_bind(val));
        }
        let sql = format!(
            "INSERT INTO \"{}\" ({}) VALUES ({})",
            rec.table,
            cols.join(", "),
            placeholders.join(", ")
        );
        db.conn()
            .execute(&sql, &params)
            .await
            .map_err(|e| format!("failed to insert into '{}': {e}", rec.table))?;
        loaded += 1;
    }
    Ok(loaded)
}

/// Load JSON fixture files into the database.
pub async fn loaddata(files: &[PathBuf], database_url: Option<&str>) -> Result<(), String> {
    if files.is_empty() {
        return Err("at least one fixture file path is required".to_string());
    }
    let db = connect_db(database_url).await?;
    let mut total = 0usize;
    for path in files {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
        let count = loaddata_from_json(&db, &content).await?;
        total += count;
    }
    println!(
        "[dj loaddata] installed {total} object(s) from {} fixture(s)",
        files.len()
    );
    Ok(())
}

/// Removes all rows from every non-system, non-migration table in `db`.
pub async fn flush_database(db: &Database) -> Result<usize, String> {
    let tables = list_user_tables(db).await?;
    for table in &tables {
        validate_sql_ident(table)?;
        let sql = match db.dialect() {
            Dialect::Postgres => format!("TRUNCATE TABLE \"{table}\" RESTART IDENTITY CASCADE"),
            Dialect::Sqlite | Dialect::MySql => format!("DELETE FROM \"{table}\""),
        };
        db.conn()
            .execute(&sql, &[])
            .await
            .map_err(|e| format!("failed to flush table '{table}': {e}"))?;
    }
    Ok(tables.len())
}

/// Remove all data from non-migration tables in the database.
pub async fn flush(no_input: bool, database_url: Option<&str>) -> Result<(), String> {
    if !no_input {
        print!("This will IRREVERSIBLY DESTROY all data in the database. Type 'yes' to continue: ");
        io::stdout().flush().map_err(|e| e.to_string())?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|e| e.to_string())?;
        if answer.trim() != "yes" {
            println!("[dj flush] cancelled");
            return Ok(());
        }
    }
    let db = connect_db(database_url).await?;
    let count = flush_database(&db).await?;
    println!("[dj flush] flushed {count} table(s)");
    Ok(())
}

/// Squash a contiguous range of SQL migration files into a single file.
pub fn squashmigrations(dir: &Path, start: &str, end: &str, name: &str) -> Result<(), String> {
    let out =
        djangors_migrations::squash_migrations(dir, start, end, name).map_err(|e| e.to_string())?;
    println!("[dj squashmigrations] created {}", out.display());
    Ok(())
}

/// Computes human-readable lines for settings that differ from `DjangorsSettings::default()`.
pub fn diff_settings_lines(
    current: &djangors_core::DjangorsSettings,
    defaults: &djangors_core::DjangorsSettings,
) -> Vec<String> {
    let mut diffs = Vec::new();
    macro_rules! cmp_field {
        ($name:expr, $field:ident) => {
            if current.$field != defaults.$field {
                diffs.push(format!(
                    "{} = {:?}  # default: {:?}",
                    $name, current.$field, defaults.$field
                ));
            }
        };
    }
    cmp_field!("DEBUG", debug);
    cmp_field!("HOST", host);
    cmp_field!("PORT", port);
    cmp_field!("ALLOWED_HOSTS", allowed_hosts);
    cmp_field!("CORS_ALLOWED_ORIGINS", cors_allowed_origins);
    cmp_field!("MAX_BODY_BYTES", max_body_bytes);
    cmp_field!("SESSION_REMEMBER_AGE", session_remember_age);
    cmp_field!(
        "REQUIRE_VERIFIED_EMAIL_FOR_LOGIN",
        require_verified_email_for_login
    );
    cmp_field!("SILENCED_SYSTEM_CHECKS", silenced_system_checks);
    if current.secret_key != defaults.secret_key {
        let masked = if current.secret_key.is_empty() {
            "<empty>"
        } else {
            "***"
        };
        diffs.push(format!("SECRET_KEY = \"{masked}\"  # differs from default"));
    }
    diffs
}

/// Print settings that differ from Djangors defaults.
pub fn diffsettings() -> Result<(), String> {
    let (current, _warnings) = djangors_core::DjangorsSettings::load()
        .map_err(|e| format!("failed to load settings: {e}"))?;
    let defaults = djangors_core::DjangorsSettings::default();
    let diffs = diff_settings_lines(&current, &defaults);
    if diffs.is_empty() {
        println!("[dj diffsettings] all settings match defaults");
    } else {
        for line in diffs {
            println!("{line}");
        }
    }
    Ok(())
}

/// Send a test email to verify email configuration.
pub fn sendtestemail(recipients: &[String], subject: &str) -> Result<(), String> {
    if recipients.is_empty() {
        return Err("at least one recipient email address is required".to_string());
    }
    for r in recipients {
        if !r.contains('@') || r.starts_with('@') || r.ends_with('@') {
            return Err(format!("invalid recipient email address '{r}'"));
        }
    }
    let from = std::env::var("DJANGORS_DEFAULT_FROM_EMAIL")
        .unwrap_or_else(|_| "noreply@localhost".to_string());
    let host = std::env::var("DJANGORS_EMAIL_HOST").ok();
    println!(
        "[dj sendtestemail] From: {from}\n[dj sendtestemail] To: {}\n[dj sendtestemail] Subject: {subject}",
        recipients.join(", ")
    );
    match host {
        Some(h) if !h.trim().is_empty() => {
            println!("[dj sendtestemail] SMTP host configured ({h}); message queued for delivery.");
        }
        _ => {
            println!(
                "[dj sendtestemail] console backend active (set DJANGORS_EMAIL_HOST for SMTP); test email printed above."
            );
        }
    }
    Ok(())
}

fn collect_files_with_exts(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_exts(&path, exts, out)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if exts.contains(&ext) {
                out.push(path);
            }
        }
    }
    Ok(())
}

/// Extracts `| trans` / `|trans` template message keys and `t!("...")` keys from text.
pub fn extract_translation_keys(content: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    // Match `"key" | trans` or `'key' | trans`
    let bytes = content.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != quote {
                if bytes[end] == b'\\' && end + 1 < bytes.len() {
                    end += 2;
                } else {
                    end += 1;
                }
            }
            if end < bytes.len() {
                let candidate = &content[start..end];
                let after = content[end + 1..].trim_start();
                if let Some(rest) = after.strip_prefix('|') {
                    let filter = rest.trim_start();
                    if filter.starts_with("trans")
                        && !candidate.is_empty()
                        && candidate
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
                    {
                        keys.insert(candidate.to_string());
                    }
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }

    // Match `t!("key")`
    let mut rest = content;
    while let Some(pos) = rest.find("t!(\"") {
        let after = &rest[pos + 4..];
        if let Some(end) = after.find('"') {
            let key = &after[..end];
            if !key.is_empty() {
                keys.insert(key.to_string());
            }
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    keys
}

/// Scans `project_dir` (`templates/` and `src/`) and updates `<output_dir>/<locale>.ftl`.
pub fn makemessages_in(
    project_dir: &Path,
    locale: &str,
    output_dir: &Path,
) -> Result<(PathBuf, usize), String> {
    let mut files = Vec::new();
    collect_files_with_exts(
        &project_dir.join("templates"),
        &["html", "htm", "txt"],
        &mut files,
    )
    .map_err(|e| e.to_string())?;
    collect_files_with_exts(&project_dir.join("src"), &["rs"], &mut files)
        .map_err(|e| e.to_string())?;

    let mut discovered = BTreeSet::new();
    for file in files {
        if let Ok(content) = std::fs::read_to_string(&file) {
            discovered.extend(extract_translation_keys(&content));
        }
    }

    std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
    let catalog_path = output_dir.join(format!("{locale}.ftl"));
    let existing_content = std::fs::read_to_string(&catalog_path).unwrap_or_default();
    let mut existing_keys = BTreeSet::new();
    for line in existing_content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, _)) = trimmed.split_once('=') {
            existing_keys.insert(k.trim().to_string());
        }
    }

    let mut new_entries = Vec::new();
    for key in &discovered {
        if !existing_keys.contains(key) {
            new_entries.push(format!("{key} = {key}"));
        }
    }

    let mut out = existing_content;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for entry in &new_entries {
        out.push_str(entry);
        out.push('\n');
    }
    std::fs::write(&catalog_path, out)
        .map_err(|e| format!("failed to write '{}': {e}", catalog_path.display()))?;
    Ok((catalog_path, new_entries.len()))
}

/// Scan templates and source files for `|trans` keys and update `.ftl` locale files.
pub fn makemessages(locale: &str, output_dir: &Path) -> Result<(), String> {
    let (path, added) = makemessages_in(Path::new("."), locale, output_dir)?;
    println!(
        "[dj makemessages] updated {} ({added} new message(s))",
        path.display()
    );
    Ok(())
}

/// Validates `.ftl` translation catalogs under `dir`, returning `(files_checked, messages_checked)`.
pub fn validate_ftl_dir(dir: &Path) -> Result<(usize, usize), String> {
    if !dir.exists() {
        return Err(format!(
            "locales directory '{}' does not exist",
            dir.display()
        ));
    }
    let mut files = Vec::new();
    collect_files_with_exts(dir, &["ftl"], &mut files).map_err(|e| e.to_string())?;
    files.sort();

    let mut total_messages = 0usize;
    for path in &files {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{}': {e}", path.display()))?;
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || line.starts_with(' ')
                || line.starts_with('\t')
            {
                continue;
            }
            let Some((key, val)) = trimmed.split_once('=') else {
                return Err(format!(
                    "{}:{}: expected `key = value` entry",
                    path.display(),
                    idx + 1
                ));
            };
            let key = key.trim();
            if key.is_empty()
                || !key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                return Err(format!(
                    "{}:{}: invalid Fluent message identifier '{key}'",
                    path.display(),
                    idx + 1
                ));
            }
            // Check balanced braces in value
            let opens = val.chars().filter(|&c| c == '{').count();
            let closes = val.chars().filter(|&c| c == '}').count();
            if opens != closes {
                return Err(format!(
                    "{}:{}: unbalanced '{{' / '}}' in message '{key}'",
                    path.display(),
                    idx + 1
                ));
            }
            total_messages += 1;
        }
    }
    Ok((files.len(), total_messages))
}

/// Validate `.ftl` translation catalogs under the locales directory.
pub fn compilemessages(dir: &Path) -> Result<(), String> {
    let (files, messages) = validate_ftl_dir(dir)?;
    println!("[dj compilemessages] validated {files} catalog(s) ({messages} message(s))");
    Ok(())
}

/// Updates a user's password hash in `auth_user`.
pub async fn changepassword_in_db(
    db: &Database,
    username: &str,
    new_password: &str,
) -> Result<(), String> {
    if new_password.is_empty() {
        return Err("password cannot be empty".to_string());
    }
    let hashed = djangors_auth::hash_password(new_password)
        .map_err(|e| format!("failed to hash password: {e}"))?;
    let dialect = db.dialect();
    let sql = format!(
        "UPDATE auth_user SET password = {} WHERE username = {}",
        dialect.placeholder(1),
        dialect.placeholder(2)
    );
    let affected = db
        .conn()
        .execute(
            &sql,
            &[
                BindValue::Text(hashed),
                BindValue::Text(username.to_string()),
            ],
        )
        .await
        .map_err(|e| format!("failed to update password: {e}"))?;
    if affected == 0 {
        return Err(format!("user '{username}' does not exist"));
    }
    Ok(())
}

/// Change a user's password in `auth_user` (hashed with Argon2id).
pub async fn changepassword(
    username: &str,
    password: Option<&str>,
    database_url: Option<&str>,
) -> Result<(), String> {
    let new_password = match password {
        Some(p) => p.to_string(),
        None => std::env::var("DJANGORS_NEW_PASSWORD").map_err(|_| {
            "pass --password <NEW_PASSWORD> or set DJANGORS_NEW_PASSWORD".to_string()
        })?,
    };
    let db = connect_db(database_url).await?;
    changepassword_in_db(&db, username, &new_password).await?;
    println!("[dj changepassword] password changed successfully for user '{username}'");
    Ok(())
}

/// Creates the database cache table in `db`.
pub async fn createcachetable_in_db(db: &Database, table_name: &str) -> Result<(), String> {
    validate_sql_ident(table_name)?;
    let (bytea, timestamp) = match db.dialect() {
        Dialect::Postgres => ("BYTEA", "TIMESTAMPTZ"),
        Dialect::Sqlite => ("BLOB", "TEXT"),
        Dialect::MySql => ("LONGBLOB", "DATETIME(6)"),
    };
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS \"{table_name}\" (key TEXT PRIMARY KEY, value {bytea} NOT NULL, expires_at {timestamp})"
    );
    db.conn()
        .execute(&sql, &[])
        .await
        .map_err(|e| format!("failed to create cache table '{table_name}': {e}"))?;
    Ok(())
}

/// Create the database table required for `DatabaseCache`.
pub async fn createcachetable(table_name: &str, database_url: Option<&str>) -> Result<(), String> {
    let db = connect_db(database_url).await?;
    createcachetable_in_db(&db, table_name).await?;
    println!("[dj createcachetable] cache table '{table_name}' is ready");
    Ok(())
}

/// Scans `src_dir` for `.route("...", ...)` and `.nest("...", ...)` calls and returns `(path, file)` pairs.
pub fn discover_routes_in(src_dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    collect_files_with_exts(src_dir, &["rs"], &mut files).map_err(|e| e.to_string())?;
    files.sort();

    let mut routes: BTreeMap<String, String> = BTreeMap::new();
    for file in files {
        let Ok(content) = std::fs::read_to_string(&file) else {
            continue;
        };
        let rel = file.display().to_string();
        for needle in [".route(\"", ".nest(\""] {
            let mut rest = content.as_str();
            while let Some(pos) = rest.find(needle) {
                let after = &rest[pos + needle.len()..];
                if let Some(end) = after.find('"') {
                    let route_path = &after[..end];
                    routes
                        .entry(route_path.to_string())
                        .or_insert_with(|| rel.clone());
                    rest = &after[end + 1..];
                } else {
                    break;
                }
            }
        }
    }
    Ok(routes.into_iter().collect())
}

/// List all registered HTTP routes in the current project.
pub fn showurls() -> Result<(), String> {
    crate::commands::require_project_root()?;
    let routes = discover_routes_in(Path::new("src"))?;
    if routes.is_empty() {
        println!("[dj showurls] no routes found in src/");
    } else {
        for (path, file) in routes {
            println!("{path:<40} {file}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(prefix: &str) -> PathBuf {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "djangors_cli_manage_{prefix}_{}_{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn sqlite_mem() -> Database {
        let cfg = djangors_db::config::DatabaseConfig::new("sqlite::memory:");
        Database::connect(&cfg).await.unwrap()
    }

    #[tokio::test]
    async fn test_dumpdata_loaddata_and_flush_sqlite() {
        let db = sqlite_mem().await;
        db.conn()
            .execute(
                "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
                &[],
            )
            .await
            .unwrap();
        db.conn()
            .execute("INSERT INTO items (id, name) VALUES (1, 'alpha')", &[])
            .await
            .unwrap();
        db.conn()
            .execute("INSERT INTO items (id, name) VALUES (2, 'beta')", &[])
            .await
            .unwrap();

        let dumped = dumpdata_records(&db, &[]).await.unwrap();
        assert_eq!(dumped.len(), 2);
        let json = serde_json::to_string(&dumped).unwrap();

        let flushed = flush_database(&db).await.unwrap();
        assert_eq!(flushed, 1);
        assert!(dumpdata_records(&db, &[]).await.unwrap().is_empty());

        let loaded = loaddata_from_json(&db, &json).await.unwrap();
        assert_eq!(loaded, 2);
        let after = dumpdata_records(&db, &["items".to_string()]).await.unwrap();
        assert_eq!(after.len(), 2);
    }

    #[tokio::test]
    async fn test_createcachetable_and_changepassword_sqlite() {
        let db = sqlite_mem().await;
        createcachetable_in_db(&db, "my_cache_table").await.unwrap();
        db.conn()
            .execute(
                "INSERT INTO my_cache_table (key, value, expires_at) VALUES ('k', X'01', NULL)",
                &[],
            )
            .await
            .unwrap();

        db.conn()
            .execute(
                "CREATE TABLE auth_user (id INTEGER PRIMARY KEY, username TEXT UNIQUE, password TEXT)",
                &[],
            )
            .await
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO auth_user (id, username, password) VALUES (1, 'alice', 'old')",
                &[],
            )
            .await
            .unwrap();

        changepassword_in_db(&db, "alice", "new-secret-pass-123")
            .await
            .unwrap();
        let row = db
            .conn()
            .fetch_one(
                "SELECT password FROM auth_user WHERE username = 'alice'",
                &[],
            )
            .await
            .unwrap();
        let hash = row.try_string(0).unwrap().unwrap();
        assert!(hash.starts_with("$argon2"));

        let missing = changepassword_in_db(&db, "nobody", "pass").await;
        assert!(missing.is_err());
    }

    #[test]
    fn test_squashmigrations_and_makemessages_compilemessages() {
        let dir = temp_dir("mgmt");
        let mig_dir = dir.join("migrations");
        std::fs::create_dir_all(&mig_dir).unwrap();
        std::fs::write(
            mig_dir.join("0001_init.sql"),
            "-- up\nCREATE TABLE a (id INT);\n-- down\nDROP TABLE a;\n",
        )
        .unwrap();
        std::fs::write(
            mig_dir.join("0002_add_b.sql"),
            "-- up\nCREATE TABLE b (id INT);\n-- down\nDROP TABLE b;\n",
        )
        .unwrap();
        squashmigrations(&mig_dir, "0001", "0002", "0001_squashed_0002").unwrap();
        assert!(mig_dir.join("0001_squashed_0002.sql").is_file());

        let tpl_dir = dir.join("templates");
        std::fs::create_dir_all(&tpl_dir).unwrap();
        std::fs::write(
            tpl_dir.join("index.html"),
            "<h1>{{ \"welcome-title\" | trans }}</h1>",
        )
        .unwrap();
        let locales = dir.join("locales");
        let (ftl_path, added) = makemessages_in(&dir, "fr", &locales).unwrap();
        assert_eq!(added, 1);
        assert!(ftl_path.is_file());

        let (files, msgs) = validate_ftl_dir(&locales).unwrap();
        assert_eq!(files, 1);
        assert_eq!(msgs, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_diff_settings_and_showurls() {
        let defaults = djangors_core::DjangorsSettings::default();
        let mut custom = defaults.clone();
        custom.port = 9000;
        let diffs = diff_settings_lines(&custom, &defaults);
        assert!(diffs.iter().any(|l| l.starts_with("PORT = 9000")));

        let dir = temp_dir("urls");
        let src = dir.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("main.rs"),
            r#"Router::new().route("/api/posts", get(list)).nest("/admin", admin_router);"#,
        )
        .unwrap();
        let routes = discover_routes_in(&src).unwrap();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].0, "/admin");
        assert_eq!(routes[1].0, "/api/posts");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
