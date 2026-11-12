use super::*;
use djangors_auth::{verify_password, User};
use djangors_db::Database;
use djangors_orm::Model as _;

static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
static FS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn superuser_test_database() -> Database {
    let sqlite = matches!(
        std::env::var("TEST_BACKEND").as_deref(),
        Ok(value) if value.eq_ignore_ascii_case("sqlite")
    );
    let config = if sqlite {
        djangors_db::config::DatabaseConfig::new("sqlite::memory:")
            .max_connections(1)
            .min_connections(1)
    } else {
        djangors_db::config::DatabaseConfig::new(
            std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/djangors_test".into()),
        )
    };
    let db = Database::connect(&config).await.unwrap();
    let mut conn = db.conn();
    conn.execute("DROP TABLE IF EXISTS auth_user", &[])
        .await
        .unwrap();
    let dialect = db.dialect();
    let id = if dialect == djangors_db::Dialect::Sqlite {
        "INTEGER PRIMARY KEY"
    } else {
        "BIGSERIAL PRIMARY KEY"
    };
    let timestamp = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE auth_user (\
                id {id}, \
                username VARCHAR(150) NOT NULL, \
                email VARCHAR(254) NOT NULL, \
                password TEXT NOT NULL, \
                is_active BOOLEAN NOT NULL, \
                is_staff BOOLEAN NOT NULL, \
                is_superuser BOOLEAN NOT NULL, \
                date_joined {timestamp} NOT NULL, \
                last_login {timestamp}\
            )"
    );
    conn.execute(&sql, &[]).await.unwrap();
    db
}

async fn drop_superuser_test_table(db: &Database) {
    db.conn()
        .execute("DROP TABLE IF EXISTS auth_user", &[])
        .await
        .unwrap();
}

// Regression tests for a real bug: `dj new`, when compiled as a standalone `cargo install
// djangors-cli` from crates.io (not run from inside the cloned monorepo), used to bake in a
// path dependency on a sibling `djangors-core` directory that only exists in a workspace
// checkout - the generated project's `cargo build` failed immediately with a genuine
// "No such file or directory". Reproduced live: packaged djangors-cli with `cargo package`
// (the exact manifest crates.io would serve, path deps already stripped by cargo itself),
// installed it, ran `dj new` from that binary, and confirmed the generated project builds
// and serves real HTTP traffic end-to-end.
#[test]
fn test_resolve_core_dependency_uses_path_when_sibling_checkout_exists() {
    let temp_dir = std::env::temp_dir().join(format!(
        "dj_test_resolve_core_dep_path_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(temp_dir.join("djangors-core")).unwrap();
    std::fs::write(
        temp_dir.join("djangors-core").join("Cargo.toml"),
        "[package]\nname = \"djangors-core\"\n",
    )
    .unwrap();

    match resolve_core_dependency_at(temp_dir.clone()) {
        CoreDependency::Path(p) => assert_eq!(p, temp_dir),
        CoreDependency::Version(v) => panic!("expected Path, got Version({v})"),
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_resolve_core_dependency_falls_back_to_version_when_no_sibling_checkout() {
    // A registry cache / packaged-crate directory never has an unversioned `djangors-core`
    // sibling (real on-disk names are always `djangors-core-X.Y.Z`), so this must fall back
    // to a version pin rather than generating a path that can never resolve.
    let temp_dir = std::env::temp_dir().join(format!(
        "dj_test_resolve_core_dep_version_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    match resolve_core_dependency_at(temp_dir.clone()) {
        CoreDependency::Version(v) => assert_eq!(v, env!("CARGO_PKG_VERSION")),
        CoreDependency::Path(p) => panic!("expected Version, got Path({})", p.display()),
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_create_superuser_success() {
    let _guard = DB_MUTEX.lock().unwrap();
    let db = superuser_test_database().await;

    let username = "admin_test_user";
    let email = "admin_test@example.com";
    let password = "super_secure_password";

    let result = create_superuser_impl(&db, username, email, password).await;
    assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result);

    // Fetch from database
    let user = User::objects()
        .filter(djangors_orm::q!(username = username))
        .unwrap()
        .get(&db)
        .await
        .unwrap();

    assert_eq!(user.username, username);
    assert_eq!(user.email, email);
    assert!(user.is_active);
    assert!(user.is_staff);
    assert!(user.is_superuser);

    // Verify password is correct using real auth verify function
    let is_valid = verify_password(password, &user.password).unwrap();
    assert!(is_valid, "Password verification failed");

    drop_superuser_test_table(&db).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_create_superuser_duplicate_username() {
    let _guard = DB_MUTEX.lock().unwrap();
    let db = superuser_test_database().await;

    let username = "admin_test_dup";
    let email1 = "admin1@example.com";
    let email2 = "admin2@example.com";
    let password = "password";

    let result1 = create_superuser_impl(&db, username, email1, password).await;
    if let Err(ref e) = result1 {
        println!("result1 error: {:?}", e);
    }
    assert!(result1.is_ok());

    let result2 = create_superuser_impl(&db, username, email2, password).await;
    assert!(
        result2.is_err(),
        "Expected Err for duplicate username, got Ok"
    );
    let err_msg = result2.unwrap_err();
    assert!(
        err_msg.contains("already exists"),
        "Err message: {}",
        err_msg
    );

    drop_superuser_test_table(&db).await;
}

#[test]
fn test_new_generates_expected_files() {
    let temp_dir = std::env::temp_dir().join(format!("dj_test_new_gen_{}", std::process::id()));
    let project_path = temp_dir.join("my_test_proj");

    let _ = std::fs::remove_dir_all(&project_path);
    let _ = std::fs::remove_dir_all(&temp_dir);

    let res = new(project_path.to_str().unwrap());
    assert!(res.is_ok(), "new() failed: {:?}", res);

    assert!(project_path.join("Cargo.toml").exists());
    assert!(project_path.join("src/main.rs").exists());
    assert!(project_path.join("src/views.rs").exists());
    assert!(project_path.join(".gitignore").exists());
    assert!(project_path.join("djangors.toml").exists());
    assert!(project_path.join("README.md").exists());
    assert!(project_path.join("Dockerfile").exists());
    assert!(project_path.join("deploy/djangors.service").exists());

    let readme_content = std::fs::read_to_string(project_path.join("README.md")).unwrap();
    assert!(readme_content.contains("## Deploying"));

    let views_content = std::fs::read_to_string(project_path.join("src/views.rs")).unwrap();
    assert!(views_content.contains("pub async fn healthz"));

    let seed_content = std::fs::read_to_string(project_path.join("src/bin/seed.rs")).unwrap();
    assert!(seed_content.contains("DATABASE_URL"));
    assert!(seed_content.contains("Nothing to seed yet"));

    let main_content = std::fs::read_to_string(project_path.join("src/main.rs")).unwrap();
    assert!(main_content.contains("/healthz"));
    // App code is in the library crate so the seed binary can use it.
    assert!(main_content.contains("use my_test_proj::views;"));
    let lib_content = std::fs::read_to_string(project_path.join("src/lib.rs")).unwrap();
    assert!(lib_content.contains("pub mod views;"));
    assert!(seed_content.contains("use my_test_proj::"));

    let cargo_toml_content = std::fs::read_to_string(project_path.join("Cargo.toml")).unwrap();
    assert!(cargo_toml_content.contains("name = \"my_test_proj\""));
    assert!(cargo_toml_content.contains("default-run = \"my_test_proj\""));
    assert!(cargo_toml_content.contains("djangors-core = { path = "));
    assert!(cargo_toml_content.contains("djangors-db = { path = "));
    assert!(cargo_toml_content.contains("[profile.release]"));
    assert!(cargo_toml_content.contains("lto = \"thin\""));
    assert!(cargo_toml_content.contains("strip = \"symbols\""));
    assert!(cargo_toml_content.contains("codegen-units = 1"));

    let path_line = cargo_toml_content
        .lines()
        .find(|l| l.contains("djangors-core ="))
        .expect("Cargo.toml missing djangors-core line");
    let start_quote = path_line.find('"').unwrap();
    let end_quote = path_line.rfind('"').unwrap();
    let extracted_core_path = &path_line[start_quote + 1..end_quote];

    assert!(
        std::path::Path::new(extracted_core_path)
            .join("Cargo.toml")
            .exists(),
        "Extracted djangors-core path '{}' does not exist",
        extracted_core_path
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_new_rejects_existing_directory() {
    let temp_dir = std::env::temp_dir().join(format!("dj_test_new_exists_{}", std::process::id()));
    let project_path = temp_dir.join("existing_proj");

    let _ = std::fs::remove_dir_all(&project_path);
    let _ = std::fs::remove_dir_all(&temp_dir);

    std::fs::create_dir_all(&project_path).unwrap();

    let res = new(project_path.to_str().unwrap());
    assert!(res.is_err());
    let err = res.unwrap_err();
    assert!(err.contains("already exists"), "Error message: {err}");

    let entry_count = std::fs::read_dir(&project_path).unwrap().count();
    assert_eq!(
        entry_count, 0,
        "Directory should remain empty after rejection"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_new_invalid_project_name() {
    assert!(new("123invalid").is_err());
    assert!(new("invalid name!").is_err());
    assert!(new("").is_err());
}

#[test]
fn test_new_rejects_seed_as_project_name() {
    // Checked before anything is written, so no directory is created.
    let err = new("seed").unwrap_err();
    assert!(err.contains("reserved"), "Error message: {err}");
}

#[test]
fn test_seed_blocked_by_debug_env_values() {
    assert!(seed_blocked_by_debug_env(Some("false")));
    assert!(seed_blocked_by_debug_env(Some("FALSE")));
    assert!(seed_blocked_by_debug_env(Some("0")));
    assert!(!seed_blocked_by_debug_env(Some("true")));
    assert!(!seed_blocked_by_debug_env(Some("1")));
    assert!(!seed_blocked_by_debug_env(None));
}

#[test]
fn test_seed_requires_seed_script() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_seed_missing_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    std::fs::remove_file(project.join("src/bin/seed.rs")).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let res = seed(false);

    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);

    let err = res.unwrap_err();
    assert!(err.contains("src/bin/seed.rs"), "Error message: {err}");
}

#[test]
fn test_seed_refused_when_debug_is_false() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_seed_debug_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let old_debug = std::env::var("DJANGORS_DEBUG").ok();
    std::env::set_var("DJANGORS_DEBUG", "false");

    let res = seed(false);

    match old_debug {
        Some(value) => std::env::set_var("DJANGORS_DEBUG", value),
        None => std::env::remove_var("DJANGORS_DEBUG"),
    }
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);

    let err = res.unwrap_err();
    assert!(err.contains("DJANGORS_DEBUG"), "Error message: {err}");
    assert!(err.contains("--force"), "Error message: {err}");
}

/// Slow because it builds a generated standalone project; run with `--ignored`.
#[test]
#[ignore]
fn test_new_app_generates_module() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_app_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();
    new_app("accounts").unwrap();
    assert!(project.join("src/accounts/mod.rs").exists());
    assert!(project.join("src/accounts/models.rs").exists());
    assert!(project.join("src/accounts/views.rs").exists());
    assert!(project.join("src/accounts/admin.rs").exists());
    std::fs::write(
        project.join("src/main.rs"),
        format!(
            "{}\nmod accounts;\n",
            std::fs::read_to_string(project.join("src/main.rs")).unwrap()
        ),
    )
    .unwrap();
    let status = std::process::Command::new("cargo")
        .args(["build", "--offline"])
        .current_dir(&project)
        .status()
        .unwrap();
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert!(status.success());
}

#[test]
fn test_check_generated_project_and_invalid_settings() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_check_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();
    assert!(check(false).unwrap().is_empty());
    std::fs::write("djangors.toml", "debug = false\n").unwrap();
    let issues = check(false).unwrap();
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert!(issues
        .iter()
        .any(|issue| issue.contains("SECRET_KEY cannot be empty")));
}

#[test]
fn test_check_deploy_clean_and_invalid() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_check_deploy_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let bad_issues = check(true).unwrap();
    assert!(bad_issues
        .iter()
        .any(|issue| issue.contains("DEBUG is true; must be false in production")));

    std::fs::write(
        "djangors.toml",
        r#"
debug = false
secret_key = "123456789012345678901234567890123"
allowed_hosts = ["example.com"]
"#,
    )
    .unwrap();

    let clean_issues = check(true).unwrap();
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert!(
        clean_issues.is_empty(),
        "Expected zero deploy issues, got: {:?}",
        clean_issues
    );
}

#[test]
fn test_makemigrations_generated_project_introspects() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_makemigrations_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let res1 = makemigrations(false);
    assert!(res1.is_ok(), "makemigrations failed: {res1:?}");

    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn test_dbshell_missing_database_url() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_dbshell_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let old_url = std::env::var("DATABASE_URL").ok();
    std::env::remove_var("DATABASE_URL");

    let res = dbshell();
    if let Some(url) = old_url {
        std::env::set_var("DATABASE_URL", url);
    }
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);

    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .contains("DATABASE_URL environment variable is not set"));
}

#[test]
fn test_dbshell_psql_uri_connection() {
    if matches!(
        std::env::var("TEST_BACKEND").as_deref(),
        Ok(value) if value.eq_ignore_ascii_case("sqlite")
    ) {
        return; // `dbshell` is intentionally a PostgreSQL `psql` integration.
    }
    let db_url = "postgres://postgres:postgres@localhost/djangors_test";
    let status = std::process::Command::new("psql")
        .arg(db_url)
        .args(["-c", "SELECT 1;"])
        .status()
        .expect("failed to execute psql");
    assert!(status.success(), "psql URI connection failed");
}

#[test]
fn test_dj_test_command() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_cmd_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let res = test();

    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);

    assert!(res.is_ok(), "test() failed: {:?}", res);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_dj_shell_missing_evcxr() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_shell_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let old_url = std::env::var("DATABASE_URL").ok();
    let db_url = if matches!(
        std::env::var("TEST_BACKEND").as_deref(),
        Ok(value) if value.eq_ignore_ascii_case("sqlite")
    ) {
        "sqlite::memory:"
    } else {
        "postgres://postgres:postgres@localhost/djangors_test"
    };
    std::env::set_var("DATABASE_URL", db_url);
    let old_path = std::env::var("PATH").ok();
    std::env::set_var("PATH", "/bin:/usr/bin");

    let res = shell().await;

    if let Some(path) = old_path {
        std::env::set_var("PATH", path);
    }
    if let Some(url) = old_url {
        std::env::set_var("DATABASE_URL", url);
    } else {
        std::env::remove_var("DATABASE_URL");
    }
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);

    assert!(res.is_err());
    assert!(res.unwrap_err().contains("failed to execute 'evcxr'"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_dj_shell_missing_db_url() {
    let _guard = FS_MUTEX.lock().unwrap();
    let root = std::env::temp_dir().join(format!("dj_test_shell_nodb_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let old = std::env::current_dir().unwrap();
    std::env::set_current_dir(&project).unwrap();

    let old_url = std::env::var("DATABASE_URL").ok();
    std::env::remove_var("DATABASE_URL");

    let res = shell().await;

    if let Some(url) = old_url {
        std::env::set_var("DATABASE_URL", url);
    }
    std::env::set_current_dir(old).unwrap();
    let _ = std::fs::remove_dir_all(root);

    assert!(res.is_err());
    assert!(res
        .unwrap_err()
        .contains("DATABASE_URL environment variable is not set"));
}

/// End-to-end proof that a generated project compiles and serves the welcome page over HTTP.
/// Run explicitly via `cargo test -p djangors-cli -- --ignored`.
#[test]
#[ignore]
fn test_new_generated_project_builds_and_serves() {
    let _guard = FS_MUTEX.lock().unwrap();
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    let temp_dir = std::env::temp_dir().join(format!("dj_test_e2e_{}", std::process::id()));
    let project_path = temp_dir.join("test_e2e_serves");

    let _ = std::fs::remove_dir_all(&project_path);
    let _ = std::fs::remove_dir_all(&temp_dir);

    let res = new(project_path.to_str().unwrap());
    assert!(res.is_ok(), "new() failed: {:?}", res);

    let build_status = std::process::Command::new("cargo")
        .args(["build", "--offline"])
        .current_dir(&project_path)
        .status()
        .expect("failed to run cargo build on generated project");
    assert!(
        build_status.success(),
        "cargo build failed for generated project"
    );

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let bin_path = project_path
        .join("target")
        .join("debug")
        .join("test_e2e_serves");
    let mut child = std::process::Command::new(&bin_path)
        .env("DJANGORS_PORT", port.to_string())
        .spawn()
        .expect("failed to spawn generated binary");

    let start = Instant::now();
    let mut stream_opt = None;
    while start.elapsed() < Duration::from_secs(15) {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream_opt = Some(stream);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut stream = match stream_opt {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            panic!("Generated server did not start and bind to port {port} within 15 seconds");
        }
    };

    let request = format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("failed to send request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("failed to read response");

    let health_req =
        format!("GET /healthz HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    let mut health_response = String::new();
    if let Ok(mut health_stream) = TcpStream::connect(("127.0.0.1", port)) {
        health_stream
            .write_all(health_req.as_bytes())
            .expect("failed to send health request");
        health_stream
            .read_to_string(&mut health_response)
            .expect("failed to read health response");
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&temp_dir);

    assert!(
        response.contains("200 OK"),
        "Expected 200 OK for /, got: {response}"
    );
    assert!(
        response.contains("It worked!"),
        "Expected 'It worked!', got: {response}"
    );
    assert!(
        health_response.contains("200 OK"),
        "Expected 200 OK for /healthz, got: {health_response}"
    );
    assert!(
        health_response.contains("ok"),
        "Expected 'ok' for /healthz body, got: {health_response}"
    );
}

/// Slow end-to-end proof of the hand-rolled build/watch/restart loop.
/// Run with `cargo test -p djangors-cli -- --ignored`.
#[test]
#[ignore]
fn test_run_rebuilds_after_source_change() {
    let _guard = FS_MUTEX.lock().unwrap();
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    let root = std::env::temp_dir().join(format!("dj_test_run_{}", std::process::id()));
    let project = root.join("project");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    new(project.to_str().unwrap()).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let mut runner = std::process::Command::new("cargo")
        .args(["run", "--manifest-path"])
        .arg(&manifest)
        .args([
            "-p",
            "djangors-cli",
            "--",
            "run",
            "--port",
            &port.to_string(),
        ])
        .current_dir(&project)
        .spawn()
        .unwrap();
    let request = |expected: &str| -> bool {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.write_all(
                "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n".as_bytes(),
            );
            let mut body = String::new();
            let _ = stream.read_to_string(&mut body);
            body.contains(expected)
        } else {
            false
        }
    };
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(90) && !request("It worked!") {
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(request("It worked!"), "watch server did not start");
    let views = project.join("src/views.rs");
    let content = std::fs::read_to_string(&views)
        .unwrap()
        .replace("It worked!", "Changed by watcher");
    std::fs::write(views, content).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(90) && !request("Changed by watcher") {
        std::thread::sleep(Duration::from_millis(250));
    }
    let changed = request("Changed by watcher");
    let _ = runner.kill();
    let _ = runner.wait();
    // `runner` is the `cargo run` process; `dj run`'s own watch loop spawns the
    // generated server binary as ITS child. A hard kill of `runner` does not
    // cascade to that grandchild (SIGKILL never propagates to descendants), so
    // it survives as an orphan holding the port (and, inheriting this test
    // process's stdio, would otherwise hang anything piping this test's output
    // waiting for EOF that never comes). Find and kill it directly by matching
    // its cmdline against this test's own unique binary path.
    kill_processes_matching(&project.join("target/debug"));
    let _ = std::fs::remove_dir_all(root);
    assert!(changed, "watch server did not serve rebuilt content");
}

/// Best-effort: SIGKILL any running process whose *resolved executable path*
/// is under `dir` (Linux-only, via /proc — acceptable since this whole test
/// suite already assumes a Linux/Postgres environment). Must check
/// `/proc/<pid>/exe` (which always resolves to the real absolute path, even
/// for a since-deleted binary), not `cmdline` — `dj run` spawns the built
/// binary via a *relative* path (`target/debug/<pkg>`, since it runs with the
/// project directory as its cwd), so `cmdline` alone would never match an
/// absolute `dir`.
fn kill_processes_matching(dir: &Path) {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let pid_str = entry.file_name();
        let Some(pid_str) = pid_str.to_str() else {
            continue;
        };
        if pid_str.parse::<u32>().is_err() {
            continue;
        }
        let exe_path = entry.path().join("exe");
        let Ok(resolved) = std::fs::read_link(&exe_path) else {
            continue;
        };
        if resolved.starts_with(dir) {
            let _ = std::process::Command::new("kill")
                .args(["-9", pid_str])
                .status();
        }
    }
}

#[test]
fn test_format_showmigrations_output() {
    let statuses = vec![
        MigrationStatus {
            name: "0001_initial".to_string(),
            applied: true,
            missing_from_disk: false,
        },
        MigrationStatus {
            name: "0002_add_field".to_string(),
            applied: false,
            missing_from_disk: false,
        },
        MigrationStatus {
            name: "0003_deleted".to_string(),
            applied: true,
            missing_from_disk: true,
        },
    ];
    let formatted = format_showmigrations("polls", &statuses);
    let expected =
        "polls\n [X] 0001_initial\n [ ] 0002_add_field\n [?] 0003_deleted (missing from disk)\n";
    assert_eq!(formatted, expected);
}

#[test]
fn test_render_openapi_typescript_client() {
    let snapshot = djangors_orm::ModelSnapshot {
        struct_name: "Post".to_string(),
        app_label: "posts".to_string(),
        table_name: "posts_post".to_string(),
        fields: vec![
            djangors_orm::FieldSnapshot {
                name: "id".to_string(),
                column_name: "id".to_string(),
                kind: djangors_orm::FieldKind::BigInt,
                nullable: false,
                primary_key: true,
                auto: true,
                unique: true,
                db_index: true,
                default: djangors_orm::SnapshotDefault::None,
                max_length: None,
            },
            djangors_orm::FieldSnapshot {
                name: "title".to_string(),
                column_name: "title".to_string(),
                kind: djangors_orm::FieldKind::Char,
                nullable: false,
                primary_key: false,
                auto: false,
                unique: false,
                db_index: false,
                default: djangors_orm::SnapshotDefault::None,
                max_length: Some(255),
            },
            djangors_orm::FieldSnapshot {
                name: "published".to_string(),
                column_name: "published".to_string(),
                kind: djangors_orm::FieldKind::Boolean,
                nullable: false,
                primary_key: false,
                auto: false,
                unique: false,
                db_index: false,
                default: djangors_orm::SnapshotDefault::Bool(false),
                max_length: None,
            },
        ],
        relations: Vec::new(),
        unique_together: Vec::new(),
        ordering: Vec::new(),
        indexes: Vec::new(),
    };

    let ts =
        render_openapi_from_snapshots(&[snapshot], true, "Blog API", "1.0.0", "/api", &[]).unwrap();
    assert!(ts.contains("export interface ProblemDetails {"));
    assert!(ts.contains("export class ProblemDetailsError extends Error {"));
    assert!(ts.contains("export interface Post {"));
    assert!(ts.contains("  id: number;"));
    assert!(ts.contains("  title: string;"));
    assert!(ts.contains("  published: boolean;"));
    assert!(ts.contains("export interface PostCreateInput {"));
    assert!(ts.contains("  id?: number;"));
    assert!(ts.contains("export class DjangorsApiClient {"));
    assert!(ts.contains("async listPost("));
    assert!(ts.contains("async createPost(data: PostCreateInput): Promise<Post>"));
}

#[test]
fn openapi_prefix_and_model_filter_are_applied() {
    let snapshot = djangors_orm::ModelSnapshot {
        struct_name: "Post".into(),
        app_label: "posts".into(),
        table_name: "posts".into(),
        fields: vec![],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };
    let other = djangors_orm::ModelSnapshot {
        struct_name: "User".into(),
        app_label: "users".into(),
        table_name: "users".into(),
        fields: vec![],
        relations: vec![],
        unique_together: vec![],
        ordering: vec![],
        indexes: vec![],
    };
    let rendered = render_openapi_from_snapshots(
        &[snapshot.clone(), other],
        false,
        "API",
        "1",
        "/v2",
        &["Post".into()],
    )
    .unwrap();
    assert!(rendered.contains("/v2/posts"));
    assert!(!rendered.contains("/v2/users"));
    assert!(rendered.contains("Paths assume each model is mounted at /v2/<plural>/."));
    let error =
        render_openapi_from_snapshots(&[snapshot], false, "API", "1", "/api", &["Missing".into()])
            .unwrap_err();
    assert!(error.contains("Missing") && error.contains("Post"));
}

#[test]
fn test_generate_resource_scaffolds_full_stack_files() {
    let temp = std::env::temp_dir().join(format!("dj_gen_resource_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(temp.join("src")).unwrap();

    let specs = vec![
        "title:string".to_string(),
        "body:text".to_string(),
        "published:bool".to_string(),
    ];
    generate_resource_in_dir(&temp, "Post", &specs, None, djangors_db::Dialect::Postgres).unwrap();

    let models = std::fs::read_to_string(temp.join("src/posts/models.rs")).unwrap();
    assert!(models.contains("pub struct Post"));
    assert!(models.contains("#[djangors(max_length = 255)]"));
    assert!(models.contains("pub title: String"));
    assert!(models.contains("pub body: String"));
    assert!(models.contains("pub published: bool"));

    let serializers = std::fs::read_to_string(temp.join("src/posts/serializers.rs")).unwrap();
    assert!(serializers.contains("pub fn post_serializer() -> ModelSerializer<Post>"));

    let views = std::fs::read_to_string(temp.join("src/posts/views.rs")).unwrap();
    assert!(views.contains("\"/api/posts\""));

    let admin = std::fs::read_to_string(temp.join("src/posts/admin.rs")).unwrap();
    assert!(admin.contains("site.register_with::<Post>"));

    let tests = std::fs::read_to_string(temp.join("src/posts/tests.rs")).unwrap();
    assert!(tests.contains("fn post_metadata_and_json_roundtrip()"));

    let mig = std::fs::read_to_string(temp.join("migrations/0001_create_posts_post.sql")).unwrap();
    assert!(mig.contains("CREATE TABLE \"posts_post\""));
    assert!(mig.contains("DROP TABLE \"posts_post\""));
    assert!(mig.contains("BIGSERIAL PRIMARY KEY"));

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_generate_resource_sqlite_migration_applies() {
    let temp = std::env::temp_dir().join(format!("dj_gen_resource_sqlite_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(temp.join("src")).unwrap();

    let specs: Vec<String> = [
        "title:string",
        "body:text",
        "published:bool",
        "views:int",
        "score:float",
        "created:datetime",
        "token:uuid",
        "price:decimal",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    generate_resource_in_dir(&temp, "Post", &specs, None, djangors_db::Dialect::Sqlite).unwrap();

    let mig = std::fs::read_to_string(temp.join("migrations/0001_create_posts_post.sql")).unwrap();
    assert!(mig.contains("INTEGER PRIMARY KEY AUTOINCREMENT"));
    assert!(!mig.contains("BIGSERIAL"));
    assert!(!mig.contains("TIMESTAMPTZ"));
    let up = mig
        .split("-- down")
        .next()
        .unwrap()
        .trim_start_matches("-- up")
        .trim();

    let config = djangors_db::config::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    db.conn().execute(up, &[]).await.unwrap();

    let _ = std::fs::remove_dir_all(&temp);
}

#[tokio::test]
async fn test_inspectdb_sqlite_generates_derive_model_structs() {
    let config = djangors_db::config::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    db.conn()
        .execute(
            "CREATE TABLE blog_article (id INTEGER PRIMARY KEY AUTOINCREMENT, title VARCHAR(120) NOT NULL, views INTEGER NOT NULL, summary TEXT)",
            &[],
        )
        .await
        .unwrap();

    let (generated, _) = inspect_database_models(&db, "blog").await.unwrap();
    assert!(generated.contains("#[djangors(app = \"blog\", table_name = \"blog_article\")]"));
    assert!(generated.contains("pub struct BlogArticle {"));
    assert!(generated.contains("#[djangors(primary_key, auto)]"));
    assert!(generated.contains("pub id: i64,"));
    assert!(generated.contains("#[djangors(max_length = 120)]"));
    assert!(generated.contains("pub title: String,"));
    assert!(generated.contains("pub views: i32,"));
    assert!(generated.contains("pub summary: Option<String>,"));
}

#[test]
fn test_live_reload_server_starts_and_broadcasts() {
    let (port, tx) = start_live_reload_server(0);
    assert!(port.is_some());
    assert!(tx.is_some());
}

#[test]
fn test_inspected_type_mapping_does_not_match_int_substrings() {
    assert_eq!(inspected_base_type("integer", false), Some("i32"));
    assert_eq!(inspected_base_type("bigint", false), Some("i64"));
    assert_eq!(inspected_base_type("integer", true), Some("i64"));
    assert_eq!(
        inspected_base_type("character varying", false),
        Some("String")
    );
    assert_eq!(inspected_base_type("VARCHAR(120)", false), Some("String"));
    assert_eq!(
        inspected_base_type("numeric(10, 2)", false),
        Some("rust_decimal::Decimal")
    );
    assert_eq!(
        inspected_base_type("timestamp without time zone", false),
        Some("chrono::DateTime<chrono::Utc>")
    );
    assert_eq!(
        inspected_base_type("time without time zone", false),
        Some("chrono::NaiveTime")
    );
    // Names containing "INT" that are not integers.
    assert_eq!(inspected_base_type("interval", false), None);
    assert_eq!(inspected_base_type("point", false), None);
    assert_eq!(inspected_base_type("jsonb", false), None);
    assert_eq!(inspected_base_type("bytea", false), None);
    assert_eq!(inspected_base_type("integer[]", false), None);
}

#[test]
fn test_rust_field_for_column_handles_reserved_and_invalid_names() {
    assert_eq!(rust_field_for_column("title"), ("title".to_string(), None));
    assert_eq!(rust_field_for_column("type"), ("r#type".to_string(), None));
    assert_eq!(
        rust_field_for_column("self"),
        ("self_".to_string(), Some("column = \"self\"".to_string()))
    );
    assert_eq!(
        rust_field_for_column("First Name"),
        (
            "first_name".to_string(),
            Some("column = \"First Name\"".to_string())
        )
    );
    assert_eq!(
        rust_field_for_column("2fa"),
        ("_2fa".to_string(), Some("column = \"2fa\"".to_string()))
    );
}

#[tokio::test]
async fn test_inspectdb_warns_on_unsupported_column_types() {
    let config = djangors_db::config::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    db.conn()
        .execute(
            "CREATE TABLE files (id INTEGER PRIMARY KEY, data BLOB NOT NULL, \"self\" TEXT)",
            &[],
        )
        .await
        .unwrap();

    let (code, warnings) = inspect_database_models(&db, "app").await.unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("files.data"));
    assert!(code.contains("#[djangors(column = \"self\")]"));
    assert!(code.contains("pub self_: Option<String>,"));
    assert!(!code.contains("TODO"));
}
