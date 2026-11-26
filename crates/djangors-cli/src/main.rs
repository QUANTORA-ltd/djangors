mod cli;
mod commands;
mod manage_commands;
mod starters;

use clap::Parser;
use cli::{Cli, Commands, GenerateCommands};

fn builtin_command_names() -> std::collections::HashSet<&'static str> {
    [
        "new",
        "new-app",
        "run",
        "check",
        "dbshell",
        "runworker",
        "migrate",
        "makemigrations",
        "sqlmigrate",
        "showmigrations",
        "createsuperuser",
        "createpermissions",
        "shell",
        "test",
        "seed",
        "collectstatic",
        "openapi",
        "generate",
        "inspectdb",
        "clearsessions",
        "rotate-encryption-keys",
        "dumpdata",
        "loaddata",
        "flush",
        "squashmigrations",
        "diffsettings",
        "sendtestemail",
        "makemessages",
        "compilemessages",
        "changepassword",
        "createcachetable",
        "showurls",
    ]
    .into_iter()
    .collect()
}

#[tokio::main]
async fn main() {
    // Intercept unknown subcommands and route them as custom management commands
    // via cargo run --quiet, mirroring the introspect_models pattern.
    let args: Vec<String> = std::env::args().collect();
    if let Some(cmd) = args.get(1) {
        let builtins = builtin_command_names();
        if !cmd.starts_with('-') && !builtins.contains(cmd.as_str()) {
            let extra: Vec<&str> = args.iter().skip(2).map(|s| s.as_str()).collect();
            let mut cmd_process = std::process::Command::new("cargo");
            cmd_process
                .args(["run", "--quiet"])
                .env("DJANGORS_RUN_COMMAND", cmd)
                .current_dir(".");
            if !extra.is_empty() {
                cmd_process.arg("--");
                cmd_process.args(&extra);
            }
            let status = cmd_process.status().unwrap_or_else(|e| {
                eprintln!("[dj] error: failed to execute cargo: {e}");
                std::process::exit(1);
            });
            if !status.success() {
                let mut list: Vec<&str> = builtins.into_iter().collect();
                list.sort();
                eprintln!("[dj] error: unknown command '{cmd}'");
                eprintln!("[dj] available commands: {}", list.join(", "));
                std::process::exit(1);
            }
            return;
        }
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::New { name, starter } => {
            if let Err(e) = commands::new_with_starter(&name, starter) {
                eprintln!("[dj new] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::NewApp { name } => {
            if let Err(e) = commands::new_app(&name) {
                eprintln!("[dj new-app] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Run { port } => {
            if let Err(e) = commands::run(port) {
                eprintln!("[dj run] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Check { deploy } => match commands::check(deploy) {
            Ok(issues) if issues.is_empty() => println!("[dj check] no issues found"),
            Ok(issues) => {
                println!("[dj check] {} issue(s) found:", issues.len());
                for issue in issues {
                    println!("- {issue}");
                }
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("[dj check] error: {e}");
                std::process::exit(1);
            }
        },
        Commands::Dbshell => {
            if let Err(e) = commands::dbshell() {
                eprintln!("[dj dbshell] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::RunWorker { poll_interval_secs } => {
            commands::runworker(poll_interval_secs).await;
        }
        Commands::Migrate {
            rollback,
            plan,
            fake,
        } => {
            commands::migrate(rollback, plan, fake).await;
        }
        Commands::Makemigrations { check } => {
            if let Err(e) = commands::makemigrations(check) {
                eprintln!("[dj makemigrations] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Sqlmigrate { app, migration } => {
            if let Err(e) = commands::sqlmigrate(&app, &migration) {
                eprintln!("[dj sqlmigrate] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Showmigrations => {
            if let Err(e) = commands::showmigrations().await {
                eprintln!("[dj showmigrations] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Createsuperuser {
            username,
            email,
            skip_validation,
        } => {
            commands::createsuperuser(username, email, skip_validation).await;
        }
        Commands::Createpermissions => {
            commands::createpermissions().await;
        }
        Commands::Shell => {
            if let Err(e) = commands::shell().await {
                eprintln!("[dj shell] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Test => {
            if let Err(e) = commands::test() {
                eprintln!("[dj test] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Seed { force } => {
            if let Err(e) = commands::seed(force) {
                eprintln!("[dj seed] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Collectstatic { source, output } => {
            commands::collectstatic(source, output);
        }
        Commands::Openapi {
            typescript,
            output,
            title,
            api_version,
            api_prefix,
            model,
            routes,
        } => {
            if let Err(e) = commands::openapi(
                typescript,
                output,
                &title,
                &api_version,
                &api_prefix,
                &model,
                routes.as_deref(),
            ) {
                eprintln!("[dj openapi] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Generate { target } => match target {
            GenerateCommands::Resource { name, fields, app } => {
                if let Err(e) = commands::generate_resource(&name, &fields, app.as_deref()) {
                    eprintln!("[dj generate resource] error: {e}");
                    std::process::exit(1);
                }
            }
        },
        Commands::Inspectdb {
            database_url,
            output,
            app,
        } => {
            if let Err(e) = commands::inspectdb(database_url, output, &app).await {
                eprintln!("[dj inspectdb] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Clearsessions { database_url } => {
            match commands::clearsessions(database_url.as_deref()).await {
                Ok(removed) => println!("[dj clearsessions] removed {removed} expired session(s)"),
                Err(e) => {
                    eprintln!("[dj clearsessions] error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::RotateEncryptionKeys {
            model,
            batch,
            database_url,
        } => {
            if let Err(e) = commands::rotate_encryption_keys(&model, batch, database_url.as_deref())
            {
                eprintln!("[dj rotate-encryption-keys] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Dumpdata {
            tables,
            output,
            database_url,
        } => {
            if let Err(e) =
                manage_commands::dumpdata(&tables, output.as_deref(), database_url.as_deref()).await
            {
                eprintln!("[dj dumpdata] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Loaddata {
            files,
            database_url,
        } => {
            if let Err(e) = manage_commands::loaddata(&files, database_url.as_deref()).await {
                eprintln!("[dj loaddata] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Flush {
            no_input,
            database_url,
        } => {
            if let Err(e) = manage_commands::flush(no_input, database_url.as_deref()).await {
                eprintln!("[dj flush] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Squashmigrations {
            start,
            end,
            name,
            dir,
        } => {
            if let Err(e) = manage_commands::squashmigrations(&dir, &start, &end, &name) {
                eprintln!("[dj squashmigrations] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Diffsettings => {
            if let Err(e) = manage_commands::diffsettings() {
                eprintln!("[dj diffsettings] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Sendtestemail {
            recipients,
            subject,
        } => {
            if let Err(e) = manage_commands::sendtestemail(&recipients, &subject) {
                eprintln!("[dj sendtestemail] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Makemessages { locale, output_dir } => {
            if let Err(e) = manage_commands::makemessages(&locale, &output_dir) {
                eprintln!("[dj makemessages] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Compilemessages { dir } => {
            if let Err(e) = manage_commands::compilemessages(&dir) {
                eprintln!("[dj compilemessages] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Changepassword {
            username,
            password,
            database_url,
        } => {
            if let Err(e) = manage_commands::changepassword(
                &username,
                password.as_deref(),
                database_url.as_deref(),
            )
            .await
            {
                eprintln!("[dj changepassword] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Createcachetable {
            table_name,
            database_url,
        } => {
            if let Err(e) =
                manage_commands::createcachetable(&table_name, database_url.as_deref()).await
            {
                eprintln!("[dj createcachetable] error: {e}");
                std::process::exit(1);
            }
        }
        Commands::Showurls => {
            if let Err(e) = manage_commands::showurls() {
                eprintln!("[dj showurls] error: {e}");
                std::process::exit(1);
            }
        }
    }
}
