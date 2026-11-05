use super::*;
use djangors_auth::{verify_password, User};
use djangors_db::Database;
use djangors_orm::Model as _;

static DB_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
static FS_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct TestProcessGroup(std::process::Child);

impl Drop for TestProcessGroup {
    fn drop(&mut self) {
        let group = format!("-{}", self.0.id());
        let _ = std::process::Command::new("kill")
            .args(["-TERM", "--", &group])
            .status();
        let _ = self.0.wait();
    }
}

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

    let main_content = std::fs::read_to_string(project_path.join("src/main.rs")).unwrap();
    assert!(main_content.contains("/healthz"));

    let cargo_toml_content = std::fs::read_to_string(project_path.join("Cargo.toml")).unwrap();
    assert!(cargo_toml_content.contains("name = \"my_test_proj\""));
    assert!(cargo_toml_content.contains("djangors-core = { path = "));

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
    use std::os::unix::process::CommandExt;
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
    std::fs::copy(
        manifest.parent().unwrap().join("Cargo.lock"),
        project.join("Cargo.lock"),
    )
    .unwrap();
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.parent().unwrap().join("target"));
    let mut command = std::process::Command::new("cargo");
    command
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
        .env("CARGO_TARGET_DIR", target_dir)
        .process_group(0);
    let runner = TestProcessGroup(command.spawn().unwrap());
    let request = |expected: &str| -> bool {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
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
    // The generated project may compile from scratch on a constrained CI runner.
    while start.elapsed() < Duration::from_secs(300) && !request("It worked!") {
        std::thread::sleep(Duration::from_millis(250));
    }
    if !request("It worked!") {
        drop(runner);
        let _ = std::fs::remove_dir_all(&root);
        panic!("watch server did not start");
    }
    let views = project.join("src/views.rs");
    let content = std::fs::read_to_string(&views)
        .unwrap()
        .replace("It worked!", "Changed by watcher");
    std::fs::write(views, content).unwrap();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(300) && !request("Changed by watcher") {
        std::thread::sleep(Duration::from_millis(250));
    }
    let changed = request("Changed by watcher");
    drop(runner);
    let _ = std::fs::remove_dir_all(root);
    assert!(changed, "watch server did not serve rebuilt content");
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
