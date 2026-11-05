//! Stub implementations for all dj subcommands.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub(crate) fn validate_project_name(name: &str) -> Result<(), String> {
    let path = std::path::Path::new(name);
    let crate_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid project name '{name}'"))?;

    if crate_name.is_empty() {
        return Err("project name cannot be empty".to_string());
    }
    if !crate_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "invalid project name '{crate_name}': project names may only contain ASCII letters, numbers, underscores, and hyphens"
        ));
    }
    if crate_name.chars().next().unwrap().is_ascii_digit() {
        return Err(format!(
            "invalid project name '{crate_name}': project name cannot start with a digit"
        ));
    }
    Ok(())
}

pub(crate) fn djangors_crates_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("djangors-cli has a parent directory")
        .to_path_buf()
}

/// How the generated project should depend on `djangors-core`.
///
/// `dj new` is compiled both ways: as `cargo run -p djangors-cli` from inside a clone of the
/// real monorepo (the workflow this project's own contributors and tests use), and as a
/// standalone `dj` binary from `cargo install djangors-cli` against the published crates.io
/// package (the workflow every real end user is expected to use, matching `pip install django`).
/// `CARGO_MANIFEST_DIR` is baked in at compile time either way, but it only points at a real
/// sibling `crates/` checkout in the first case - from a crates.io install it resolves to the
/// registry cache, where the actual on-disk directory is named `djangors-core-X.Y.Z` (with the
/// version suffix), not `djangors-core`, so a path dependency there is always broken. Check at
/// runtime whether that sibling directory genuinely exists (not just trusting the compile-time
/// path blindly) and fall back to a real version dependency pinned to this exact `djangors-cli`
/// build's own version - since every crate in the workspace shares one version number, that pin
/// is always the right one.
pub(crate) enum CoreDependency {
    /// A contributor running from inside the cloned monorepo: pick up local changes immediately.
    Path(std::path::PathBuf),
    /// A real end user's `cargo install djangors-cli`: pin to the matching published version.
    Version(&'static str),
}

pub(crate) fn resolve_core_dependency() -> CoreDependency {
    resolve_core_dependency_at(djangors_crates_dir())
}

/// The testable half of [`resolve_core_dependency`]: takes the candidate crates directory as a
/// parameter instead of hardcoding the compile-time path, so both branches (a real sibling
/// checkout present vs. absent) can be exercised directly against a real temp directory.
fn resolve_core_dependency_at(crates_dir: std::path::PathBuf) -> CoreDependency {
    if crates_dir
        .join("djangors-core")
        .join("Cargo.toml")
        .is_file()
    {
        CoreDependency::Path(crates_dir)
    } else {
        CoreDependency::Version(env!("CARGO_PKG_VERSION"))
    }
}

pub(crate) fn require_project_root() -> Result<(), String> {
    if !Path::new("Cargo.toml").is_file() || !Path::new("djangors.toml").is_file() {
        return Err("not a Djangors project (expected Cargo.toml and djangors.toml)".into());
    }
    Ok(())
}

/// Create a new Djangors project.
pub fn new(name: &str) -> Result<(), String> {
    let target_dir = std::path::Path::new(name);
    let crate_name = target_dir
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid project name '{name}'"))?;

    validate_project_name(name)?;

    if target_dir.exists() {
        return Err(format!("destination '{name}' already exists"));
    }

    let djangors_core_dep = match resolve_core_dependency() {
        CoreDependency::Path(crates_dir) => {
            format!(
                "{{ path = \"{}/djangors-core\" }}",
                crates_dir.to_string_lossy()
            )
        }
        CoreDependency::Version(version) => format!("\"{version}\""),
    };

    let cargo_toml = format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "{crate_name}"
path = "src/main.rs"

[dependencies]
djangors-core = {djangors_core_dep}
tokio = {{ version = "1", features = ["full"] }}
tower = {{ version = "0.5", features = ["util"] }}
"#
    );

    let main_rs = r#"use djangors_core::{Djangors, DjangorsError, DjangorsSettings, Router};

mod views;

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors_core::introspect_models_if_requested();
    djangors_core::run_management_command_if_requested().await;
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let router = Router::new()
        .get("/", views::welcome)
        .get("/healthz", views::healthz);
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

    let service = tower::ServiceBuilder::new()
        .layer(djangors_core::middleware::security_headers_layer())
        .service(router_service);

    Djangors::new(settings, Router::new()).run_service(service).await
}
"#;

    let views_rs = r#"use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

pub async fn welcome(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::html(
        StatusCode::OK,
        "<!DOCTYPE html><html><head><title>Djangors</title></head><body>\
<h1>It worked!</h1><p>Congratulations on your first Djangors project.</p>\
</body></html>"
            .to_string(),
    ))
}

pub async fn healthz(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "ok"))
}
"#;

    let gitignore = "/target\n*.env\n";

    let djangors_toml = "debug = true\n";

    let dockerfile = format!(
        r#"FROM rust:1-slim-bookworm AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/{crate_name} /app/server
EXPOSE 8000
ENV DJANGORS_PORT=8000
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:8000/healthz || exit 1
CMD ["/app/server"]
"#
    );

    let systemd_service = format!(
        r#"[Unit]
Description=Djangors Web Application ({crate_name})
After=network.target postgresql.service

[Service]
Type=simple
User=www-data
Group=www-data
WorkingDirectory=/var/www/{crate_name}
ExecStart=/var/www/{crate_name}/{crate_name}
Restart=on-failure
RestartSec=5s
EnvironmentFile=/etc/{crate_name}/djangors.env

[Install]
WantedBy=multi-user.target
"#
    );

    let readme = format!(
        r#"# {crate_name}

A Djangors project.

## Run it

    cargo run

Then visit http://127.0.0.1:8000/.

## Deploying

- **Dockerfile**: Includes a multi-stage build (`rust:1-slim` builder + `debian:bookworm-slim` runtime) with a `HEALTHCHECK` probing `/healthz`.
- **systemd**: See `deploy/djangors.service` for a sample unit file template.
- **Pre-flight Check**: Run `dj check --deploy` to verify production settings before deploying.

## Next steps

- Add models: see `examples/school` in the djangors repo for a real worked example
  (`#[derive(Model)]`, `AdminSite`, migrations-free `CREATE TABLE` in tests).
- Add the admin: `djangors-admin`'s `AdminSite` needs a `djangors_db::Database` in router state —
  see `examples/polls/src/main.rs` for the full wiring (DB connection, sessions, CSRF).
"#
    );

    std::fs::create_dir_all(target_dir.join("src"))
        .map_err(|e| format!("failed to create directory '{name}/src': {e}"))?;

    std::fs::create_dir_all(target_dir.join("deploy"))
        .map_err(|e| format!("failed to create directory '{name}/deploy': {e}"))?;

    std::fs::write(target_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| format!("failed to write '{name}/Cargo.toml': {e}"))?;

    std::fs::write(target_dir.join("src/main.rs"), main_rs)
        .map_err(|e| format!("failed to write '{name}/src/main.rs': {e}"))?;

    std::fs::write(target_dir.join("src/views.rs"), views_rs)
        .map_err(|e| format!("failed to write '{name}/src/views.rs': {e}"))?;

    std::fs::write(target_dir.join(".gitignore"), gitignore)
        .map_err(|e| format!("failed to write '{name}/.gitignore': {e}"))?;

    std::fs::write(target_dir.join("djangors.toml"), djangors_toml)
        .map_err(|e| format!("failed to write '{name}/djangors.toml': {e}"))?;

    std::fs::write(target_dir.join("Dockerfile"), dockerfile)
        .map_err(|e| format!("failed to write '{name}/Dockerfile': {e}"))?;

    std::fs::write(target_dir.join("deploy/djangors.service"), systemd_service)
        .map_err(|e| format!("failed to write '{name}/deploy/djangors.service': {e}"))?;

    std::fs::write(target_dir.join("README.md"), readme)
        .map_err(|e| format!("failed to write '{name}/README.md': {e}"))?;

    let git_res = std::process::Command::new("git")
        .arg("init")
        .current_dir(target_dir)
        .output();

    if git_res.is_err() || !git_res.as_ref().unwrap().status.success() {
        eprintln!(
            "warning: 'git init' failed (is git installed?), skipping — the project files were still created successfully"
        );
    }

    println!("[dj new] created '{name}'");
    println!("[dj new] next: cd {name} && cargo run");

    Ok(())
}

/// Create a new app within a project.
///
/// # Design Choice: Module over Crate
/// In `djangors`, the CLI command `dj new` generates a single standalone project crate rather
/// than a multi-crate workspace. Performing file surgeries automatically on a project's
/// `Cargo.toml` to dynamically convert a single-crate project into a workspace and configure
/// dependencies on the fly is highly fragile and prone to breaking user configurations.
/// Instead, `dj new-app` implements a deliberate design choice: it generates the app as a
/// sub-module inside the current project's `src/` directory (creating `src/<app_name>/mod.rs`,
/// `models.rs`, `views.rs`, `admin.rs`). This maps directly to standard Django apps, which are
/// python subpackages, and allows immediate compilation without needing workspace coordination.
/// Users are then prompted to perform the simple manual wiring steps (adding `mod <name>;` to
/// their `src/main.rs` and mounting the routes).
pub fn new_app(name: &str) -> Result<(), String> {
    require_project_root()?;
    validate_project_name(name)?;
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("invalid app name '{name}': Rust module names may only contain ASCII letters, numbers, and underscores"));
    }
    if name.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        return Err(format!(
            "invalid app name '{name}': app names cannot start with a digit"
        ));
    }
    let dir = Path::new("src").join(name);
    if dir.exists() {
        return Err(format!("destination 'src/{name}' already exists"));
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create '{}': {e}", dir.display()))?;
    let files = [
        (
            "mod.rs",
            "pub mod admin;\npub mod models;\npub mod views;\n".to_string(),
        ),
        (
            "models.rs",
            r#"//! Models for this app.
//!
//! Uncomment and adapt this example after adding the required model dependencies.
// use djangors_macros::Model;
//
// #[derive(Model, Debug, Clone)]
// #[djangors(app = "my_app", table_name = "my_app_student")]
// pub struct Student {
//     #[djangors(primary_key, auto)]
//     pub id: i64,
//     #[djangors(max_length = 100)]
//     pub first_name: String,
//     #[djangors(max_length = 100)]
//     pub last_name: String,
//     #[djangors(max_length = 254, unique)]
//     pub email: String,
//     pub is_active: bool,
// }
"#
            .to_string(),
        ),
        (
            "views.rs",
            r#"use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};

pub async fn index(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "Welcome to this Djangors app"))
}
"#
            .to_string(),
        ),
        (
            "admin.rs",
            r#"//! Admin registrations for this app.
//!
//! Example once a model is enabled in models.rs:
// use djangors_admin::AdminSite;
// use crate::my_app::models::Student;
// site.register::<Student>();
"#
            .to_string(),
        ),
    ];
    for (file, content) in files {
        std::fs::write(dir.join(file), content)
            .map_err(|e| format!("failed to write 'src/{name}/{file}': {e}"))?;
    }
    println!("[dj new-app] created 'src/{name}/'");
    println!("[dj new-app] next: add 'mod {name};' to src/main.rs, and mount its routes");
    Ok(())
}

/// Start the dev server.
pub fn run(port: u16) -> Result<(), String> {
    require_project_root()?;
    let manifest = std::fs::read_to_string("Cargo.toml").map_err(|e| e.to_string())?;
    let value: toml::Value =
        toml::from_str(&manifest).map_err(|e| format!("invalid Cargo.toml: {e}"))?;
    let package = value
        .get("package")
        .and_then(|v| v.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "Cargo.toml has no [package].name".to_string())?
        .to_string();
    let source = Path::new("src");
    let mut child = None;
    let mut needs_build = true;
    let mut last_mtime = project_mtime(source).max(file_mtime(Path::new("Cargo.toml")));
    // v1 has no signal-handling dependency in the workspace; Ctrl-C may leave the child running.
    loop {
        if child.is_none() && needs_build {
            println!("[dj run] building...");
            let status = std::process::Command::new("cargo")
                .arg("build")
                .status()
                .map_err(|e| format!("failed to run cargo build: {e}"))?;
            needs_build = false;
            if status.success() {
                let target_dir = std::env::var_os("CARGO_TARGET_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("target"));
                let binary = target_dir.join("debug").join(&package);
                let spawned = std::process::Command::new(binary)
                    .env("DJANGORS_PORT", port.to_string())
                    .spawn();
                match spawned {
                    Ok(process) => {
                        child = Some(process);
                        println!("[dj run] serving on http://127.0.0.1:{port}");
                    }
                    Err(e) => eprintln!("[dj run] failed to start binary: {e}"),
                }
            } else {
                println!("[dj run] build failed, watching for changes...");
            }
        }
        std::thread::sleep(Duration::from_millis(500));
        let current = project_mtime(source).max(file_mtime(Path::new("Cargo.toml")));
        if current > last_mtime {
            last_mtime = current;
            println!("[dj run] change detected, rebuilding...");
            if let Some(mut process) = child.take() {
                let _ = process.kill();
                let _ = process.wait();
            }
            needs_build = true;
        }
    }
}

fn file_mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn project_mtime(path: &Path) -> SystemTime {
    let mut newest = file_mtime(path);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            let mtime = if p.is_dir() {
                project_mtime(&p)
            } else if p.extension().is_some_and(|e| e == "rs") {
                file_mtime(&p)
            } else {
                SystemTime::UNIX_EPOCH
            };
            if mtime > newest {
                newest = mtime;
            }
        }
    }
    newest
}

/// Check externally observable project settings and structure.
///
/// # Limitations on Introspection
/// Unlike Django's `manage.py check`, which is generated per-project and runs within
/// the project's own execution context with direct imports to the project's codebase,
/// the global `dj` CLI tool is a standalone, compiled binary. It runs externally and
/// has no generic mechanism to dynamic-link or introspect the user's compiled models,
/// admin registries, or database schemas.
/// Therefore, `dj check` is restricted to validation that can be performed externally:
/// parsing and validating the settings (`djangors.toml`), checking file structure (e.g.
/// `Cargo.toml` and `src/main.rs`), and checking deployment readiness. Model/admin checks
/// must run inside the target binary itself.
pub fn check(deploy: bool) -> Result<Vec<String>, String> {
    require_project_root()?;
    let mut issues = Vec::new();
    let (settings_opt, warnings) = match djangors_core::DjangorsSettings::load() {
        Ok((settings, warnings)) => (Some(settings), warnings),
        Err(e) => {
            issues.push(e.to_string());
            (None, Vec::new())
        }
    };
    issues.extend(
        warnings
            .into_iter()
            .map(|w| format!("settings warning: {w}")),
    );

    if let Some(ref settings) = settings_opt {
        if let Err(e) = settings.validate() {
            issues.push(e.to_string());
        }

        if deploy {
            if settings.debug {
                issues.push("DEBUG is true; must be false in production".into());
            }
            if settings.secret_key.len() < 32 && (!settings.secret_key.is_empty() || settings.debug)
            {
                issues.push("SECRET_KEY should be at least 32 characters".into());
            }
            let default_hosts = vec!["127.0.0.1".to_string(), "localhost".to_string()];
            if !settings.debug && settings.allowed_hosts == default_hosts {
                issues.push(
                    "ALLOWED_HOSTS looks like the default; set it explicitly for production".into(),
                );
            }
        }
    }

    let manifest_path = Path::new("Cargo.toml");
    if !manifest_path.is_file() {
        issues.push("Cargo.toml is missing".into());
    } else {
        let content = std::fs::read_to_string(manifest_path).map_err(|e| e.to_string())?;
        let value: toml::Value =
            toml::from_str(&content).map_err(|e| format!("invalid Cargo.toml: {e}"))?;
        if value
            .get("bin")
            .and_then(toml::Value::as_array)
            .is_none_or(|bins| bins.is_empty())
        {
            issues.push("Cargo.toml has no [[bin]] table".into());
        }
    }
    if !Path::new("src/main.rs").is_file() {
        issues.push("src/main.rs is missing".into());
    }
    Ok(issues)
}

/// Status of a migration for `showmigrations`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MigrationStatus {
    /// Migration name (e.g., `0001_initial`).
    pub name: String,
    /// Whether recorded in `djangors_migrations` history table.
    pub applied: bool,
    /// Whether missing from disk despite being in history table.
    pub missing_from_disk: bool,
}

/// Formats the migration statuses into Django-style output.
pub fn format_showmigrations(app_name: &str, statuses: &[MigrationStatus]) -> String {
    let mut out = String::new();
    out.push_str(app_name);
    out.push('\n');
    for status in statuses {
        if status.missing_from_disk {
            out.push_str(&format!(" [?] {} (missing from disk)\n", status.name));
        } else if status.applied {
            out.push_str(&format!(" [X] {}\n", status.name));
        } else {
            out.push_str(&format!(" [ ] {}\n", status.name));
        }
    }
    out
}

fn get_app_name() -> Option<String> {
    let manifest_path = Path::new("Cargo.toml");
    if manifest_path.is_file() {
        let content = std::fs::read_to_string(manifest_path).ok()?;
        let value: toml::Value = toml::from_str(&content).ok()?;
        value
            .get("package")?
            .get("name")?
            .as_str()
            .map(String::from)
    } else {
        None
    }
}

/// List all available migrations and their applied status.
pub async fn showmigrations() -> Result<(), String> {
    require_project_root()?;
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            return Err("DATABASE_URL environment variable is not set".to_string());
        }
    };

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            return Err(format!("failed to connect to database: {e}"));
        }
    };

    let applied_names = djangors_migrations::get_applied_migrations(&db)
        .await
        .map_err(|e| e.to_string())?;

    let applied_set: std::collections::HashSet<String> = applied_names.into_iter().collect();

    let mut disk_names = Vec::new();
    let migrations_dir = Path::new("migrations");
    if migrations_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(migrations_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("sql") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if stem.get(..4).and_then(|s| s.parse::<u32>().ok()).is_some() {
                            disk_names.push(stem.to_string());
                        }
                    }
                }
            }
        }
    }

    disk_names.sort();

    let mut all_names = disk_names.clone();
    for name in &applied_set {
        if !all_names.contains(name) {
            all_names.push(name.clone());
        }
    }
    all_names.sort();

    let disk_set: std::collections::HashSet<String> = disk_names.into_iter().collect();

    let statuses: Vec<MigrationStatus> = all_names
        .into_iter()
        .map(|name| MigrationStatus {
            applied: applied_set.contains(&name),
            missing_from_disk: applied_set.contains(&name) && !disk_set.contains(&name),
            name,
        })
        .collect();

    let app_name = get_app_name().unwrap_or_else(|| "default".to_string());
    print!("{}", format_showmigrations(&app_name, &statuses));

    Ok(())
}

fn section_up(text: &str) -> String {
    let mut up_lines = Vec::new();
    let mut in_up = false;
    for line in text.lines() {
        if line.trim() == "-- up" {
            in_up = true;
            continue;
        }
        if line.trim() == "-- down" {
            break;
        }
        if in_up {
            up_lines.push(line);
        }
    }
    up_lines.join("\n")
}

/// Render the SQL statements for a migration without executing them.
pub fn sqlmigrate(app: &str, migration: &str) -> Result<(), String> {
    require_project_root()?;

    let dialect =
        djangors_db::Dialect::from_url(&std::env::var("DATABASE_URL").unwrap_or_default());

    let migrations_dir = Path::new("migrations");
    if !migrations_dir.is_dir() {
        return Err("migrations directory does not exist".to_string());
    }

    let mut found_file: Option<PathBuf> = None;

    let app_dir = migrations_dir.join(app);
    let dirs_to_search = if app_dir.is_dir() {
        vec![app_dir, migrations_dir.to_path_buf()]
    } else {
        vec![migrations_dir.to_path_buf()]
    };

    for dir in &dirs_to_search {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("sql") {
                    if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        if stem == migration
                            || stem.starts_with(&format!("{migration}_"))
                            || file_name == format!("{migration}.sql")
                        {
                            found_file = Some(path);
                            break;
                        }
                    }
                }
            }
        }
        if found_file.is_some() {
            break;
        }
    }

    if found_file.is_none() {
        return Err(format!(
            "migration '{migration}' for app '{app}' not found in migrations directory"
        ));
    }

    let snapshot_path = Path::new("migrations/.schema_snapshot.json");
    let snapshots: Option<Vec<djangors_orm::ModelSnapshot>> = if snapshot_path.exists() {
        std::fs::read_to_string(snapshot_path)
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
    } else {
        None
    };

    let operations = if let Some(snaps) = snapshots {
        djangors_migrations::build_create_plan_from_snapshots(&snaps, dialect).ok()
    } else if let Ok(models) = introspect_models() {
        djangors_migrations::build_create_plan_from_snapshots(&models, dialect).ok()
    } else {
        None
    };

    if let Some(ops) = operations {
        for op in ops {
            match op.to_sql(dialect) {
                Ok(sql) => {
                    let trimmed = sql.trim();
                    if trimmed.ends_with(';') {
                        println!("{trimmed}");
                    } else {
                        println!("{trimmed};");
                    }
                }
                Err(djangors_migrations::MigrationError::UnsupportedOnDialect {
                    operation,
                    dialect,
                }) => {
                    println!("-- Operation '{operation}' is unsupported on dialect {dialect:?}");
                }
                Err(e) => {
                    println!("-- Error rendering SQL: {e}");
                }
            }
        }
    } else if let Some(file_path) = found_file {
        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("failed to read migration file: {e}"))?;
        let up_section = section_up(&content);
        for stmt in up_section
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            println!("{stmt};");
        }
    }

    Ok(())
}

/// Apply database migrations.
///
/// Reads the database connection URL from the `DATABASE_URL` environment
/// variable (the standard sqlx/Rust ecosystem convention) and runs
/// [`djangors_migrations::migrate`].
pub async fn migrate(rollback: Option<u32>, plan: bool, fake: bool) {
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj migrate] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj migrate] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };

    if let Some(count) = rollback {
        match djangors_migrations::rollback_from_dir(&db, Path::new("migrations"), count).await {
            Ok(()) => println!("[dj migrate] rollback completed"),
            Err(e) => {
                eprintln!("[dj migrate] rollback failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    if plan {
        let applied_names = match djangors_migrations::get_applied_migrations(&db).await {
            Ok(names) => names,
            Err(e) => {
                eprintln!("[dj migrate] failed to read migration history: {e}");
                std::process::exit(1);
            }
        };
        let applied_set: std::collections::HashSet<String> = applied_names.into_iter().collect();

        if Path::new("migrations").is_dir() {
            let mut unapplied = Vec::new();
            if let Ok(entries) = std::fs::read_dir("migrations") {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("sql") {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            if stem.get(..4).and_then(|s| s.parse::<u32>().ok()).is_some()
                                && !applied_set.contains(stem)
                            {
                                unapplied.push(stem.to_string());
                            }
                        }
                    }
                }
            }
            unapplied.sort();
            if unapplied.is_empty() {
                println!("[dj migrate] Database is up to date. No migrations to apply.");
            } else {
                println!("[dj migrate] Planned migrations:");
                for name in unapplied {
                    println!("  Apply {name}");
                }
            }
        } else if applied_set.contains("0001_initial") {
            println!("[dj migrate] Database is up to date. No migrations to apply.");
        } else {
            println!("[dj migrate] Planned migrations:");
            println!("  Apply 0001_initial");
        }
        return;
    }

    if fake {
        let applied_names = match djangors_migrations::get_applied_migrations(&db).await {
            Ok(names) => names,
            Err(e) => {
                eprintln!("[dj migrate] failed to read migration history: {e}");
                std::process::exit(1);
            }
        };
        let applied_set: std::collections::HashSet<String> = applied_names.into_iter().collect();

        if Path::new("migrations").is_dir() {
            let mut to_fake = Vec::new();
            if let Ok(entries) = std::fs::read_dir("migrations") {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("sql") {
                        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                            if stem.get(..4).and_then(|s| s.parse::<u32>().ok()).is_some()
                                && !applied_set.contains(stem)
                            {
                                to_fake.push(stem.to_string());
                            }
                        }
                    }
                }
            }
            to_fake.sort();
            if to_fake.is_empty() {
                println!("[dj migrate] Database is up to date. No migrations to fake.");
            } else {
                for name in to_fake {
                    if let Err(e) = djangors_migrations::record_migration(&db, &name).await {
                        eprintln!("[dj migrate] failed to fake migration {name}: {e}");
                        std::process::exit(1);
                    }
                    println!("[dj migrate] Marked as applied (faked): {name}");
                }
            }
        } else if applied_set.contains("0001_initial") {
            println!("[dj migrate] Database is up to date. No migrations to fake.");
        } else {
            if let Err(e) = djangors_migrations::record_migration(&db, "0001_initial").await {
                eprintln!("[dj migrate] failed to fake migration 0001_initial: {e}");
                std::process::exit(1);
            }
            println!("[dj migrate] Marked as applied (faked): 0001_initial");
        }
        return;
    }

    let result = if Path::new("migrations").is_dir() {
        djangors_migrations::migrate_from_dir(&db, Path::new("migrations")).await
    } else {
        let models = match introspect_models() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[dj migrate] introspection failed: {e}");
                std::process::exit(1);
            }
        };
        migrate_with_plan(&db, &models).await
    };
    match result {
        Ok(()) => println!("[dj migrate] migrations applied successfully"),
        Err(e) => {
            eprintln!("[dj migrate] migration failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Connects to DATABASE_URL and runs the background task worker.
pub async fn runworker(poll_interval_secs: u64) {
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj runworker] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj runworker] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };
    djangors_tasks::Worker::new(db)
        .with_poll_interval(Duration::from_secs(poll_interval_secs))
        .with_recurring_tick_interval(Duration::from_secs(poll_interval_secs))
        .run()
        .await;
}

/// Generate new migrations.
pub fn makemigrations(_check: bool) -> Result<(), String> {
    require_project_root()?;
    let check = _check;
    let dialect =
        djangors_db::Dialect::from_url(&std::env::var("DATABASE_URL").unwrap_or_default());
    let current = introspect_models()?;
    let path = Path::new("migrations/.schema_snapshot.json");
    let previous: Vec<djangors_orm::ModelSnapshot> = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("invalid schema snapshot: {e}"))?
    } else {
        Vec::new()
    };
    let mut sql = Vec::new();
    let mut down = Vec::new();

    // New models must be planned together, not one at a time: a foreign key from a
    // new model to another new model (e.g. djangors-auth's UserGroup -> User) can
    // only be resolved if the target's own snapshot is in the same slice.
    let new_models: Vec<djangors_orm::ModelSnapshot> = current
        .iter()
        .filter(|model| !previous.iter().any(|m| m.table_name == model.table_name))
        .cloned()
        .collect();
    if !new_models.is_empty() {
        for op in djangors_migrations::build_create_plan_from_snapshots(&new_models, dialect)
            .map_err(|e| e.to_string())?
        {
            let sql_str = op.to_sql(dialect).map_err(|e| e.to_string())?;
            sql.push(format!("{};", sql_str));
            let down_str = match op.to_down_sql(dialect) {
                Some(Ok(s)) => format!("{};", s),
                Some(Err(e)) => return Err(e.to_string()),
                None => "".to_string(),
            };
            down.push(down_str);
        }
    }

    for model in &current {
        let old = previous.iter().find(|m| m.table_name == model.table_name);
        if let Some(old) = old {
            for field in &model.fields {
                if !old
                    .fields
                    .iter()
                    .any(|f| f.column_name == field.column_name)
                {
                    let sql_type = djangors_migrations::type_mapping::sql_type_for(
                        &field.kind,
                        field.max_length,
                        field.auto,
                        &field.name,
                        dialect,
                    )
                    .map_err(|e| e.to_string())?;
                    let op = djangors_migrations::Operation::AddColumn {
                        table_name: model.table_name.clone(),
                        column: djangors_migrations::ColumnDef {
                            name: field.column_name.clone(),
                            nullable: field.nullable,
                            sql_type,
                            primary_key: field.primary_key,
                            unique: field.unique,
                            default_sql: None,
                            references: None,
                        },
                    };
                    let sql_str = op.to_sql(dialect).map_err(|e| e.to_string())?;
                    sql.push(sql_str);
                    let down_str = match op.to_down_sql(dialect) {
                        Some(Ok(s)) => s,
                        Some(Err(e)) => return Err(e.to_string()),
                        None => "".to_string(),
                    };
                    down.push(down_str);
                }
            }
        }
    }
    if sql.is_empty() {
        if check {
            return Ok(());
        }
        println!("[dj makemigrations] no changes detected");
        return Ok(());
    }
    if check {
        return Err("model changes detected".into());
    }
    std::fs::create_dir_all("migrations").map_err(|e| e.to_string())?;
    let n = std::fs::read_dir("migrations")
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|s| s.get(..4))
                .and_then(|n| n.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
        + 1;
    let body = format!("-- up\n{}\n-- down\n{}", sql.join("\n"), down.join("\n"));
    std::fs::write(format!("migrations/{n:04}_auto.sql"), body).map_err(|e| e.to_string())?;
    std::fs::write(path, serde_json::to_string_pretty(&current).unwrap())
        .map_err(|e| e.to_string())?;
    println!("[dj makemigrations] generated {n:04}_auto.sql");
    Ok(())
}

fn introspect_models() -> Result<Vec<djangors_orm::ModelSnapshot>, String> {
    let out = std::process::Command::new("cargo")
        .args(["run", "--quiet"])
        .env("DJANGORS_INTROSPECT_MODELS", "1")
        .current_dir(".")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("invalid model metadata: {e}"))
}

async fn migrate_with_plan(
    db: &djangors_db::Database,
    models: &[djangors_orm::ModelSnapshot],
) -> Result<(), djangors_migrations::MigrationError> {
    let dialect = db.dialect();
    let history_sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_migrations (\
            id {}, \
            name TEXT UNIQUE NOT NULL, \
            applied_at {} NOT NULL DEFAULT {}\
        )",
        dialect.auto_pk_type(),
        dialect.timestamp_type(),
        dialect.current_timestamp()
    );
    db.conn().execute(&history_sql, &[]).await?;

    let check_sql = format!(
        "SELECT 1 FROM djangors_migrations WHERE name = {}",
        dialect.placeholder(1)
    );
    let params = [djangors_db::BindValue::Text("0001_initial".to_string())];
    if db
        .conn()
        .fetch_optional(&check_sql, &params)
        .await?
        .is_some()
    {
        return Ok(());
    }

    let plan = djangors_migrations::build_create_plan_from_snapshots(models, dialect)?;
    let mut sqls = Vec::new();
    for o in &plan {
        sqls.push(o.to_sql(dialect)?);
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
            conn.execute(
                &insert_sql,
                &[djangors_db::BindValue::Text("0001_initial".to_string())],
            )
            .await?;
            Ok::<(), djangors_db::DbError>(())
        })
    })
    .await
    .map_err(djangors_migrations::MigrationError::Database)
}

/// Create a superuser in the database.
///
/// This function executes the core logic of creating a superuser:
/// 1. Checks if the username is already taken.
/// 2. Hashes the password using djangors_auth::hash_password.
/// 3. Saves the User model to the database.
pub async fn create_superuser_impl(
    db: &djangors_db::Database,
    username: &str,
    email: &str,
    password: &str,
) -> Result<(), String> {
    use djangors_orm::Model as _;

    let exists = djangors_auth::User::objects()
        .filter(djangors_orm::q!(username = username))
        .map_err(|e| e.to_string())?
        .exists(db)
        .await
        .map_err(|e| e.to_string())?;

    if exists {
        return Err(format!(
            "superuser with username '{}' already exists",
            username
        ));
    }

    let hash = djangors_auth::hash_password(password).map_err(|e| e.to_string())?;

    let user = djangors_auth::User {
        id: 0,
        username: username.to_string(),
        email: email.to_string(),
        password: hash,
        is_active: true,
        is_staff: true,
        is_superuser: true,
        date_joined: chrono::Utc::now(),
        last_login: None,
    };

    user.save(db).await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Create an admin user.
///
/// Note: non-interactive only, mirrors Django's `DJANGO_SUPERUSER_PASSWORD` convention;
/// interactive prompting deferred.
pub async fn createsuperuser(username: String, email: String) {
    let password = match std::env::var("DJANGORS_SUPERUSER_PASSWORD") {
        Ok(val) if !val.is_empty() => val,
        _ => {
            eprintln!("[dj createsuperuser] DJANGORS_SUPERUSER_PASSWORD environment variable is not set or empty");
            std::process::exit(1);
        }
    };

    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj createsuperuser] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj createsuperuser] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };

    match create_superuser_impl(&db, &username, &email, &password).await {
        Ok(()) => {
            println!("Superuser created successfully: {}", username);
        }
        Err(e) => {
            eprintln!("[dj createsuperuser] failed to create superuser: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn createpermissions() {
    let db_url = match std::env::var("DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("[dj createpermissions] DATABASE_URL environment variable is not set");
            std::process::exit(1);
        }
    };
    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let db = match djangors_db::Database::connect(&config).await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("[dj createpermissions] failed to connect to database: {e}");
            std::process::exit(1);
        }
    };
    match djangors_auth::sync_permissions(&db).await {
        Ok(count) => match djangors_contrib_contenttypes::sync_content_types(&db).await {
            Ok(content_types) => println!(
                "[dj createpermissions] synced {} permission(s) and {} content type(s)",
                count, content_types
            ),
            Err(e) => {
                eprintln!("[dj createpermissions] failed: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("[dj createpermissions] failed: {e}");
            std::process::exit(1);
        }
    }
}

/// Open an interactive database shell using psql.
pub fn dbshell() -> Result<(), String> {
    require_project_root()?;
    let db_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL environment variable is not set".to_string())?;

    let status = std::process::Command::new("psql")
        .arg(&db_url)
        .status()
        .map_err(|e| format!("failed to execute 'psql' (is psql installed and on PATH?): {e}"))?;

    if !status.success() {
        return Err(format!("psql exited with status {status}"));
    }
    Ok(())
}

/// Connect to DATABASE_URL and launch interactive Rust REPL via evcxr.
pub async fn shell() -> Result<(), String> {
    require_project_root()?;
    let db_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL environment variable is not set".to_string())?;

    let config = djangors_db::config::DatabaseConfig::new(db_url);
    let _db = djangors_db::Database::connect(&config)
        .await
        .map_err(|e| format!("failed to connect to database: {e}"))?;

    println!("[dj shell] Connected to database successfully.");

    let pkg_name = std::fs::read_to_string("Cargo.toml")
        .ok()
        .and_then(|content| toml::from_str::<toml::Value>(&content).ok())
        .and_then(|val| val.get("package")?.get("name")?.as_str().map(String::from))
        .unwrap_or_else(|| "<project_name>".to_string());

    println!("[dj shell] Launching interactive Rust REPL (evcxr)...");
    println!("[dj shell] Note: Project models are not auto-imported across binary boundaries.");
    println!("[dj shell] To load your project in the REPL, run:");
    println!("[dj shell]   :dep {pkg_name} = {{ path = \".\" }}");
    println!("[dj shell]   use {pkg_name}::models::*;");

    let status = std::process::Command::new("evcxr")
        .current_dir(".")
        .status()
        .map_err(|e| format!("failed to execute 'evcxr' (is evcxr installed via 'cargo install evcxr_repl' and on PATH?): {e}"))?;

    if !status.success() {
        return Err(format!("evcxr exited with status {status}"));
    }
    Ok(())
}

/// Run the test suite.
pub fn test() -> Result<(), String> {
    require_project_root()?;
    let status = std::process::Command::new("cargo")
        .arg("test")
        .status()
        .map_err(|e| format!("failed to run cargo test: {e}"))?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Collect static files from source directories into one output directory.
pub fn collectstatic(source: Vec<PathBuf>, output: PathBuf) {
    let sources = if source.is_empty() {
        vec![PathBuf::from("static")]
    } else {
        source
    };

    let sf = djangors_staticfiles::StaticFiles::new(sources);
    match sf.collect(&output) {
        Ok(manifest) => {
            println!(
                "[dj collectstatic] collected {} file(s) into {}",
                manifest.mapping.len(),
                output.display()
            );
        }
        Err(e) => {
            eprintln!("[dj collectstatic] failed: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
