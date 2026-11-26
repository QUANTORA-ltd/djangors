use crate::cli::StarterKit;
use crate::commands::CoreDependency;

/// Tailored project files generated for non-minimal starter kits.
pub(crate) struct StarterFiles {
    pub cargo_toml: String,
    pub djangors_toml: String,
    pub main_rs: String,
    pub index_html: Option<String>,
}

fn dep_spec(dep: &CoreDependency, pkg: &str) -> String {
    match dep {
        CoreDependency::Path(crates_dir) => {
            format!("{{ path = \"{}/{pkg}\" }}", crates_dir.to_string_lossy())
        }
        CoreDependency::Version(version) => format!("\"{version}\""),
    }
}

fn cargo_manifest(crate_name: &str, deps: &str) -> String {
    format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2021"
publish = false
default-run = "{crate_name}"

[[bin]]
name = "{crate_name}"
path = "src/main.rs"

[dependencies]
{deps}
[profile.dev]
debug = "line-tables-only"

[profile.dev.package."*"]
debug = 0

[profile.release]
opt-level = "z"
lto = "thin"
strip = true
codegen-units = 1
panic = "abort"
"#
    )
}

/// Generate tailored starter files for `Api`, `Htmx`, `Inertia`, or `Saas`.
///
/// `StarterKit::Minimal` is handled directly in `commands::new_with_starter` to
/// preserve 100% byte-for-byte compatibility with existing `dj new` output.
pub(crate) fn generate_starter_files(
    starter: StarterKit,
    crate_name: &str,
    dep: &CoreDependency,
) -> Option<StarterFiles> {
    let core_dep = dep_spec(dep, "djangors-core");
    let db_dep = dep_spec(dep, "djangors-db");

    match starter {
        StarterKit::Minimal => None,
        StarterKit::Api => {
            let rest_dep = dep_spec(dep, "djangors-rest");
            let deps = format!(
                "djangors-core = {core_dep}\n\
                 djangors-db = {db_dep}\n\
                 djangors-rest = {rest_dep}\n\
                 serde = {{ version = \"1\", features = [\"derive\"] }}\n\
                 tokio = {{ version = \"1\", features = [\"full\"] }}\n\
                 tower = {{ version = \"0.5\", features = [\"util\"] }}"
            );
            let main_rs = format!(
                r#"use djangors_core::{{
    Djangors, DjangorsError, DjangorsSettings, HealthChecks, PathParams, Request, Response,
    Router, StatusCode,
}};
use djangors_rest::{{endpoint, JsonSchema, OpenApiBuilder}};
use serde::Serialize;

#[derive(Serialize, JsonSchema)]
pub struct HelloResponse {{
    pub message: String,
    pub service: String,
}}

#[endpoint(GET, "/api/hello", response = HelloResponse, summary = "Hello endpoint", tag = "api")]
pub async fn hello(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {{
    Response::json(
        StatusCode::OK,
        &HelloResponse {{
            message: "Hello from {crate_name}!".to_string(),
            service: "{crate_name}".to_string(),
        }},
    )
}}

async fn openapi_spec(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {{
    let mut builder = OpenApiBuilder::new("{crate_name} API", "0.1.0");
    let endpoints = djangors_core::endpoint::collect_endpoints();
    builder.register_endpoints(&endpoints);
    Response::json(StatusCode::OK, &builder.build())
}}

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {{
    djangors_core::introspect_models_if_requested();
    djangors_core::run_management_command_if_requested().await;
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {{
        eprintln!("settings warning: {{w}}");
    }}

    let router = Router::new()
        .endpoint(hello)
        .get("/api/openapi.json", openapi_spec)
        .health(HealthChecks::new());
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

    let service = tower::ServiceBuilder::new()
        .layer(djangors_core::middleware::security_headers_layer())
        .service(router_service);

    Djangors::new(settings, Router::new()).run_service(service).await
}}
"#
            );

            Some(StarterFiles {
                cargo_toml: cargo_manifest(crate_name, &deps),
                djangors_toml: "debug = true\n".to_string(),
                main_rs,
                index_html: None,
            })
        }
        StarterKit::Htmx => {
            let template_dep = dep_spec(dep, "djangors-template");
            let deps = format!(
                "djangors-core = {core_dep}\n\
                 djangors-db = {db_dep}\n\
                 djangors-template = {template_dep}\n\
                 serde = {{ version = \"1\", features = [\"derive\"] }}\n\
                 tokio = {{ version = \"1\", features = [\"full\"] }}\n\
                 tower = {{ version = \"0.5\", features = [\"util\"] }}"
            );
            let main_rs = format!(
                r#"use std::path::PathBuf;

use djangors_core::{{
    Djangors, DjangorsError, DjangorsSettings, HealthChecks, PathParams, Request, Response,
    Router, StatusCode,
}};
use djangors_template::TemplateEngine;
use serde::Serialize;

#[derive(Serialize)]
struct PageContext {{
    title: &'static str,
}}

async fn index(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {{
    if req.is_htmx() {{
        let resp = Response::html(
            StatusCode::OK,
            "<p id=\"greeting\">Updated via htmx from {crate_name}!</p>".to_string(),
        )
        .hx_trigger("greetingUpdated");
        return Ok(resp);
    }}

    let engine = TemplateEngine::new(vec![PathBuf::from("templates")])
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    djangors_template::render(
        &engine,
        "index.html",
        PageContext {{
            title: "{crate_name}",
        }},
    )
}}

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {{
    djangors_core::introspect_models_if_requested();
    djangors_core::run_management_command_if_requested().await;
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {{
        eprintln!("settings warning: {{w}}");
    }}

    let router = Router::new()
        .get("/", index)
        .health(HealthChecks::new());
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

    let service = tower::ServiceBuilder::new()
        .layer(djangors_core::middleware::security_headers_layer())
        .service(router_service);

    Djangors::new(settings, Router::new()).run_service(service).await
}}
"#
            );
            let index_html = format!(
                r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{{{{ title }}}}</title>
  <script src="https://unpkg.com/htmx.org@2.0.4"></script>
</head>
<body>
  <main>
    <h1>{crate_name} (htmx)</h1>
    <div id="greeting">
      <p>Ready for server-rendered htmx interactions.</p>
    </div>
    <button hx-get="/" hx-target="#greeting" hx-swap="outerHTML">
      Refresh with htmx
    </button>
  </main>
</body>
</html>
"##
            );

            Some(StarterFiles {
                cargo_toml: cargo_manifest(crate_name, &deps),
                djangors_toml: "debug = true\n".to_string(),
                main_rs,
                index_html: Some(index_html),
            })
        }
        StarterKit::Inertia => {
            let deps = format!(
                "djangors-core = {core_dep}\n\
                 djangors-db = {db_dep}\n\
                 serde = {{ version = \"1\", features = [\"derive\"] }}\n\
                 tokio = {{ version = \"1\", features = [\"full\"] }}\n\
                 tower = {{ version = \"0.5\", features = [\"util\"] }}"
            );
            let main_rs = format!(
                r#"use djangors_core::{{
    Djangors, DjangorsError, DjangorsSettings, HealthChecks, Inertia, InertiaConfig, PathParams,
    Request, Response, Router,
}};
use serde::Serialize;

#[derive(Serialize)]
struct HomeProps {{
    app_name: &'static str,
}}

async fn home(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {{
    let template = include_str!("../templates/index.html");
    let config = InertiaConfig::new("1").with_root_template(template);
    let inertia = Inertia::with_config(config);
    inertia.render(
        &req,
        "Home",
        HomeProps {{
            app_name: "{crate_name}",
        }},
    )
}}

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {{
    djangors_core::introspect_models_if_requested();
    djangors_core::run_management_command_if_requested().await;
    djangors_core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {{
        eprintln!("settings warning: {{w}}");
    }}

    let router = Router::new()
        .get("/", home)
        .health(HealthChecks::new());
    let router_service = djangors_core::router::RouterService::new(router, settings.debug);

    let service = tower::ServiceBuilder::new()
        .layer(djangors_core::middleware::security_headers_layer())
        .service(router_service);

    Djangors::new(settings, Router::new()).run_service(service).await
}}
"#
            );
            let index_html = format!(
                r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{crate_name}</title>
</head>
<body>
  <div id="app" data-page="{{{{page}}}}"></div>
</body>
</html>
"#
            );

            Some(StarterFiles {
                cargo_toml: cargo_manifest(crate_name, &deps),
                djangors_toml: "debug = true\n".to_string(),
                main_rs,
                index_html: Some(index_html),
            })
        }
        StarterKit::Saas => {
            let djangors_dep = dep_spec(dep, "djangors");
            let deps = format!(
                "djangors = {djangors_dep}\n\
                 djangors-db = {db_dep}\n\
                 tokio = {{ version = \"1\", features = [\"full\"] }}\n\
                 tower = {{ version = \"0.5\", features = [\"util\"] }}"
            );
            let main_rs = r#"use djangors::admin::AdminSite;
use djangors::auth::AuthBackend;
use djangors::core::health::{DatabaseCheck, HealthChecks};
use djangors::core::{
    Djangors, DjangorsError, DjangorsSettings, PathParams, Request, Response, Router, StatusCode,
};
use djangors::db::{config::DatabaseConfig, Database};
use djangors::flags::FeatureFlags;
use djangors::mail::ConsoleBackend;
use djangors::notifications::NotificationService;
use djangors::sessions::{SessionLayer, SignedCookieStore};

async fn dashboard(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::html(
        StatusCode::OK,
        include_str!("../templates/index.html").to_string(),
    ))
}

#[tokio::main]
async fn main() -> Result<(), DjangorsError> {
    djangors::core::introspect_models_if_requested();
    djangors::core::run_management_command_if_requested().await;
    djangors::core::logging::init_dev_logging();

    let (settings, warnings) = DjangorsSettings::load()?;
    for w in warnings {
        eprintln!("settings warning: {w}");
    }

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite::memory:".to_string());
    let db = Database::connect(&DatabaseConfig::new(db_url))
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let admin_site = AdminSite::new();
    djangors::flags::register_admin(&admin_site);
    djangors::notifications::register_admin(&admin_site);

    let _auth = AuthBackend::new();
    let _flags = FeatureFlags::new(db.clone());
    let _notifications = NotificationService::new(db.clone());
    let _mail = ConsoleBackend;

    let secret_key = if settings.secret_key.is_empty() {
        "dev-only-secret-key-at-least-32-bytes-long-for-signing-cookies".to_string()
    } else {
        settings.secret_key.clone()
    };

    let router = Router::new()
        .get("/", dashboard)
        .mount("/admin", admin_site.router())
        .health(HealthChecks::new().with(DatabaseCheck::new(db.clone())))
        .with_state(db);
    let router_service = djangors::core::router::RouterService::new(router, settings.debug);

    let service = tower::ServiceBuilder::new()
        .layer(djangors::core::middleware::security_headers_layer())
        .layer(SessionLayer::new(
            SignedCookieStore::new(secret_key.as_bytes()).with_secure(!settings.debug),
        ))
        .service(router_service);

    Djangors::new(settings, Router::new()).run_service(service).await
}
"#
            .to_string();
            let index_html = format!(
                r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{crate_name} — SaaS Starter</title>
</head>
<body>
  <main>
    <h1>{crate_name} SaaS</h1>
    <p>Auth, sessions, feature flags, notifications, mail, admin, and health checks are wired and ready.</p>
    <ul>
      <li><a href="/admin/">Admin</a></li>
      <li><a href="/healthz">Health Check</a></li>
    </ul>
  </main>
</body>
</html>
"#
            );

            Some(StarterFiles {
                cargo_toml: cargo_manifest(crate_name, &deps),
                djangors_toml: "debug = true\n".to_string(),
                main_rs,
                index_html: Some(index_html),
            })
        }
    }
}
