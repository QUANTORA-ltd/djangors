use super::*;

#[test]
fn test_parse_new() {
    let args = vec!["dj", "new", "myproject"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::New { name } => {
            assert_eq!(name, "myproject");
        }
        _ => panic!("Expected Commands::New"),
    }
}

#[test]
fn test_parse_run_with_port() {
    let args = vec!["dj", "run", "--port", "9000"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run { port } => {
            assert_eq!(port, 9000);
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_parse_run_default_port() {
    let args = vec!["dj", "run"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Run { port } => {
            assert_eq!(port, 8000);
        }
        _ => panic!("Expected Commands::Run"),
    }
}

#[test]
fn test_parse_makemigrations_check() {
    let args = vec!["dj", "makemigrations", "--check"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Makemigrations { check } => {
            assert!(check);
        }
        _ => panic!("Expected Commands::Makemigrations"),
    }
}

#[test]
fn test_parse_createsuperuser() {
    let args = vec![
        "dj",
        "createsuperuser",
        "--username",
        "admin",
        "--email",
        "admin@example.com",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Createsuperuser { username, email } => {
            assert_eq!(username, "admin");
            assert_eq!(email, "admin@example.com");
        }
        _ => panic!("Expected Commands::Createsuperuser"),
    }
}

#[test]
fn test_parse_check_deploy() {
    let args = vec!["dj", "check", "--deploy"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Check { deploy } => {
            assert!(deploy);
        }
        _ => panic!("Expected Commands::Check"),
    }
}

#[test]
fn test_parse_dbshell() {
    let args = vec!["dj", "dbshell"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Dbshell => {}
        _ => panic!("Expected Commands::Dbshell"),
    }
}

#[test]
fn test_parse_sqlmigrate() {
    let args = vec!["dj", "sqlmigrate", "polls", "0001"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Sqlmigrate { app, migration } => {
            assert_eq!(app, "polls");
            assert_eq!(migration, "0001");
        }
        _ => panic!("Expected Commands::Sqlmigrate"),
    }
}

#[test]
fn test_parse_showmigrations() {
    let args = vec!["dj", "showmigrations"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Showmigrations => {}
        _ => panic!("Expected Commands::Showmigrations"),
    }
}

#[test]
fn test_parse_migrate_plan() {
    let args = vec!["dj", "migrate", "--plan"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Migrate {
            rollback,
            plan,
            fake,
        } => {
            assert_eq!(rollback, None);
            assert!(plan);
            assert!(!fake);
        }
        _ => panic!("Expected Commands::Migrate"),
    }
}

#[test]
fn test_parse_migrate_fake() {
    let args = vec!["dj", "migrate", "--fake"];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Migrate {
            rollback,
            plan,
            fake,
        } => {
            assert_eq!(rollback, None);
            assert!(!plan);
            assert!(fake);
        }
        _ => panic!("Expected Commands::Migrate"),
    }
}

#[test]
fn test_migrate_plan_and_fake_conflict() {
    let args = vec!["dj", "migrate", "--plan", "--fake"];
    assert!(Cli::try_parse_from(args).is_err());
}

#[test]
fn test_migrate_rollback_and_fake_conflict() {
    let args = vec!["dj", "migrate", "--rollback", "--fake"];
    assert!(Cli::try_parse_from(args).is_err());
}

#[test]
fn test_migrate_rollback_and_plan_conflict() {
    let args = vec!["dj", "migrate", "--rollback", "--plan"];
    assert!(Cli::try_parse_from(args).is_err());
}

#[test]
fn test_parse_openapi_typescript() {
    let args = vec![
        "dj",
        "openapi",
        "--typescript",
        "--output",
        "client.ts",
        "--title",
        "Blog API",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Openapi {
            typescript,
            output,
            title,
            api_version,
            api_prefix,
            model,
        } => {
            assert!(typescript);
            assert_eq!(output, Some(PathBuf::from("client.ts")));
            assert_eq!(title, "Blog API");
            assert_eq!(api_version, "0.1.0");
            assert_eq!(api_prefix, "/api");
            assert!(model.is_empty());
        }
        _ => panic!("Expected Commands::Openapi"),
    }
}

#[test]
fn test_parse_openapi_prefix_and_models() {
    let cli = Cli::try_parse_from([
        "dj",
        "openapi",
        "--api-prefix",
        "/v2",
        "--model",
        "Post",
        "--model",
        "User",
    ])
    .unwrap();
    match cli.command {
        Commands::Openapi {
            api_prefix, model, ..
        } => {
            assert_eq!(api_prefix, "/v2");
            assert_eq!(model, vec!["Post".to_string(), "User".to_string()]);
        }
        _ => panic!("Expected Commands::Openapi"),
    }
}

#[test]
fn test_parse_generate_resource() {
    let args = vec![
        "dj",
        "generate",
        "resource",
        "Post",
        "title:string",
        "body:text",
        "published:bool",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Generate {
            target: GenerateCommands::Resource { name, fields, app },
        } => {
            assert_eq!(name, "Post");
            assert_eq!(fields, vec!["title:string", "body:text", "published:bool"]);
            assert!(app.is_none());
        }
        _ => panic!("Expected Commands::Generate(Resource)"),
    }
}

#[test]
fn test_parse_inspectdb() {
    let args = vec![
        "dj",
        "inspectdb",
        "--database-url",
        "sqlite::memory:",
        "--app",
        "legacy",
    ];
    let cli = Cli::try_parse_from(args).unwrap();
    match cli.command {
        Commands::Inspectdb {
            database_url,
            output,
            app,
        } => {
            assert_eq!(database_url.as_deref(), Some("sqlite::memory:"));
            assert!(output.is_none());
            assert_eq!(app, "legacy");
        }
        _ => panic!("Expected Commands::Inspectdb"),
    }
}

#[test]
fn test_parse_seed() {
    let cli = Cli::try_parse_from(["dj", "seed"]).unwrap();
    match cli.command {
        Commands::Seed { force } => assert!(!force),
        _ => panic!("Expected Commands::Seed"),
    }
}

#[test]
fn test_parse_seed_force() {
    let cli = Cli::try_parse_from(["dj", "seed", "--force"]).unwrap();
    match cli.command {
        Commands::Seed { force } => assert!(force),
        _ => panic!("Expected Commands::Seed"),
    }
}
