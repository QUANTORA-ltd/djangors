use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// The command-line interface for the Djangors web framework.
#[derive(Parser, Debug)]
#[command(
    name = "dj",
    version,
    about = "Djangors CLI",
    long_about = "The dj command-line tool for scaffolding, running, and managing Djangors projects."
)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands for the dj binary.
#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Create a new Djangors project.
    #[command(name = "new")]
    New {
        /// The name of the new project.
        name: String,
    },
    /// Create a new app within a project.
    #[command(name = "new-app")]
    NewApp {
        /// The name of the new app.
        name: String,
    },
    /// Start the dev server.
    #[command(name = "run")]
    Run {
        /// The port to listen on.
        #[arg(long, default_value_t = 8000)]
        port: u16,
    },
    /// Check project settings and structure.
    #[command(name = "check")]
    Check {
        /// Run production deployment pre-flight checks.
        #[arg(long)]
        deploy: bool,
    },
    /// Open an interactive database shell using psql.
    #[command(name = "dbshell")]
    Dbshell,
    /// Run the background task worker loop.
    #[command(name = "runworker")]
    RunWorker {
        /// Polling interval in seconds for claiming due tasks.
        #[arg(long, default_value_t = 1)]
        poll_interval_secs: u64,
    },
    /// Apply database migrations.
    #[command(name = "migrate")]
    Migrate {
        /// Roll back the most recent migration, or the last N migrations.
        #[arg(
            long,
            num_args = 0..=1,
            default_missing_value = "1",
            conflicts_with_all = ["plan", "fake"]
        )]
        rollback: Option<u32>,

        /// Print the ordered list of migrations that would be applied without executing them.
        #[arg(long, conflicts_with_all = ["rollback", "fake"])]
        plan: bool,

        /// Mark migrations as applied in the history table without executing their SQL.
        /// WARNING: This can silently desynchronise the migration history table from the actual database schema. Use with caution.
        #[arg(long, conflicts_with_all = ["rollback", "plan"])]
        fake: bool,
    },
    /// Generate new migrations.
    #[command(name = "makemigrations")]
    Makemigrations {
        /// Check for changes without writing them.
        #[arg(long)]
        check: bool,
    },
    /// Render the SQL statements for a migration without executing them.
    #[command(name = "sqlmigrate")]
    Sqlmigrate {
        /// The app label.
        app: String,
        /// The migration name or prefix.
        migration: String,
    },
    /// List all available migrations and their applied status.
    #[command(name = "showmigrations")]
    Showmigrations,
    /// Create an admin user.
    #[command(name = "createsuperuser")]
    Createsuperuser {
        /// The username for the superuser.
        #[arg(long)]
        username: String,

        /// The email address for the superuser.
        #[arg(long, default_value = "")]
        email: String,

        /// Skip password complexity validation.
        #[arg(long)]
        skip_validation: bool,
    },
    /// Create/update the standard view/add/change/delete permissions for every registered model.
    #[command(name = "createpermissions")]
    Createpermissions,
    /// Open a REPL.
    #[command(name = "shell")]
    Shell,
    /// Run the test suite.
    #[command(name = "test")]
    Test,
    /// Fill the development database by running the project's seed script (src/bin/seed.rs).
    #[command(name = "seed")]
    Seed {
        /// Seed even when DJANGORS_DEBUG is false (for example, a production database).
        #[arg(long)]
        force: bool,
    },
    /// Collect static files from source directories into one output directory.
    #[command(name = "collectstatic")]
    Collectstatic {
        /// Source directory to collect from. May be repeated; earlier
        /// directories win on a filename collision. Defaults to "static".
        #[arg(long = "source")]
        source: Vec<PathBuf>,
        /// Output directory for the collected, hashed files and manifest.json.
        #[arg(long, default_value = "staticfiles")]
        output: PathBuf,
    },
    /// Generate an OpenAPI 3.1 JSON specification or a typed TypeScript fetch client.
    #[command(name = "openapi")]
    Openapi {
        /// Emit TypeScript types and a typed fetch client instead of raw OpenAPI JSON.
        #[arg(long)]
        typescript: bool,
        /// Optional output file path (writes to stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// API title used in the generated OpenAPI specification.
        #[arg(long, default_value = "Djangors API")]
        title: String,
        /// API version used in the generated OpenAPI specification.
        #[arg(long, default_value = "0.1.0")]
        api_version: String,
        /// API path prefix used for every model.
        #[arg(long, default_value = "/api")]
        api_prefix: String,
        /// Include only the named model (repeatable; defaults to all models).
        #[arg(long = "model")]
        model: Vec<String>,
        /// JSON file mapping models to their real mount path and CRUD operations:
        /// `{"Post": {"path": "/api/v1/blog/posts", "ops": ["list", "retrieve"]}}`.
        /// Mapped models get exactly those paths/operations (`--api-prefix` is
        /// ignored); unmapped models get TypeScript types but no client methods.
        #[arg(long)]
        routes: Option<PathBuf>,
    },
    /// Generate code artifacts such as full-stack CRUD resources (`dj generate resource`).
    #[command(name = "generate")]
    Generate {
        /// Generator subcommand.
        #[command(subcommand)]
        target: GenerateCommands,
    },
    /// Introspect an existing database schema and emit `#[derive(Model)]` Rust structs.
    #[command(name = "inspectdb")]
    Inspectdb {
        /// Optional database URL override (defaults to `DATABASE_URL` environment variable).
        #[arg(long)]
        database_url: Option<String>,
        /// Optional output file path (writes to stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// App label applied to generated `#[djangors(app = "...")]` attributes.
        #[arg(long, default_value = "app")]
        app: String,
    },
    /// Delete expired server-side sessions from `djangors_session`.
    #[command(name = "clearsessions")]
    Clearsessions {
        /// Optional database URL override (defaults to `DATABASE_URL` environment variable).
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Re-encrypt `#[djangors(encrypted)]` fields using the first key in `DJANGORS_FIELD_ENCRYPTION_KEYS`.
    #[command(name = "rotate-encryption-keys")]
    RotateEncryptionKeys {
        /// Model name as `app.Model` (e.g. `billing.PaymentMethod`).
        #[arg(long)]
        model: String,
        /// Number of rows processed per database transaction (default: 500).
        #[arg(long, default_value_t = 500)]
        batch: usize,
        /// Optional database URL override (defaults to `DATABASE_URL` environment variable).
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Export database table rows as Django-style JSON fixtures.
    #[command(name = "dumpdata")]
    Dumpdata {
        /// Optional table names to dump (dumps all non-system tables if omitted).
        tables: Vec<String>,
        /// Optional output file path (writes to stdout if omitted).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Optional database URL override.
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Load JSON fixture files into the database.
    #[command(name = "loaddata")]
    Loaddata {
        /// Path(s) to JSON fixture files.
        files: Vec<PathBuf>,
        /// Optional database URL override.
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Remove all data from non-migration tables in the database.
    #[command(name = "flush")]
    Flush {
        /// Skip confirmation prompt.
        #[arg(long = "no-input", alias = "yes")]
        no_input: bool,
        /// Optional database URL override.
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Squash a contiguous range of SQL migration files into a single file.
    #[command(name = "squashmigrations")]
    Squashmigrations {
        /// Starting migration prefix (e.g. `0001`).
        start: String,
        /// Ending migration prefix (e.g. `0005`).
        end: String,
        /// Output squashed migration name (e.g. `0001_squashed_0005`).
        #[arg(long, default_value = "0001_squashed")]
        name: String,
        /// Migrations directory (default: `migrations`).
        #[arg(long, default_value = "migrations")]
        dir: PathBuf,
    },
    /// Print settings that differ from Djangors defaults.
    #[command(name = "diffsettings")]
    Diffsettings,
    /// Send a test email to verify email configuration.
    #[command(name = "sendtestemail")]
    Sendtestemail {
        /// Recipient email address(es).
        recipients: Vec<String>,
        /// Subject line for the test email.
        #[arg(long, default_value = "Djangors test email")]
        subject: String,
    },
    /// Scan templates and source files for `|trans` keys and update `.ftl` locale files.
    #[command(name = "makemessages")]
    Makemessages {
        /// Target locale code (e.g. `fr`, `es`, `de`).
        #[arg(short, long, default_value = "en")]
        locale: String,
        /// Output directory for `.ftl` catalog files.
        #[arg(long, default_value = "locales")]
        output_dir: PathBuf,
    },
    /// Validate `.ftl` translation catalogs under the locales directory.
    #[command(name = "compilemessages")]
    Compilemessages {
        /// Directory containing `.ftl` catalog files.
        #[arg(long, default_value = "locales")]
        dir: PathBuf,
    },
    /// Change a user's password in `auth_user` (hashed with Argon2id).
    #[command(name = "changepassword")]
    Changepassword {
        /// Username to update.
        username: String,
        /// New password (if omitted, reads from `DJANGORS_NEW_PASSWORD` env var).
        #[arg(long)]
        password: Option<String>,
        /// Optional database URL override.
        #[arg(long)]
        database_url: Option<String>,
    },
    /// Create the database table required for `DatabaseCache`.
    #[command(name = "createcachetable")]
    Createcachetable {
        /// Cache table name (default: `djangors_cache`).
        #[arg(default_value = "djangors_cache")]
        table_name: String,
        /// Optional database URL override.
        #[arg(long)]
        database_url: Option<String>,
    },
    /// List all registered HTTP routes in the current project.
    #[command(name = "showurls")]
    Showurls,
}

/// Subcommands for `dj generate`.
#[derive(Subcommand, Debug, Clone)]
pub enum GenerateCommands {
    /// Scaffold a model, SQL migration, REST serializer + ViewSet, admin registration, and test file.
    #[command(name = "resource")]
    Resource {
        /// PascalCase model name (e.g. `Post` or `BlogComment`).
        name: String,
        /// Field specifications in `name:type` format (e.g. `title:string body:text published:bool`).
        fields: Vec<String>,
        /// App module name under `src/<app>/` (defaults to the snake_case plural of `name`).
        #[arg(long)]
        app: Option<String>,
    },
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
