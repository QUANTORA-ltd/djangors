//! Automatic model and schema documentation views (`django.contrib.admindocs` equivalent).
//!
//! Mounted under `/doc/` on an [`AdminSite`](crate::AdminSite):
//! - `GET /doc/` — Documentation index.
//! - `GET /doc/models/` — List of all registered models, tables, fields, relations, and constraints.
//! - `GET /doc/models/{app}/{model}/` — Detailed field, relation, index, and constraint reference.

use crate::perms::require_staff;
use crate::{SiteBranding, ADMIN_TEMPLATES};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_orm::meta::ModelMeta;

#[derive(serde::Serialize)]
struct AdminDocsIndexContext {
    model_count: usize,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

#[derive(serde::Serialize)]
struct AdminDocsModelSummary {
    app_label: String,
    struct_name: String,
    model_slug: String,
    table_name: String,
    field_names: Vec<String>,
    relation_names: Vec<String>,
    constraint_descriptions: Vec<String>,
}

#[derive(serde::Serialize)]
struct AdminDocsModelsContext {
    models: Vec<AdminDocsModelSummary>,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

#[derive(serde::Serialize)]
struct AdminDocsFieldRow {
    name: String,
    column_name: String,
    kind: String,
    nullable: bool,
    primary_key: bool,
    unique: bool,
    db_index: bool,
    max_length: Option<u32>,
    choices: Vec<String>,
    help_text: String,
}

#[derive(serde::Serialize)]
struct AdminDocsRelationRow {
    field_name: String,
    kind: String,
    target: String,
    on_delete: String,
    related_name: String,
}

#[derive(serde::Serialize)]
struct AdminDocsIndexRow {
    name: String,
    fields: String,
}

#[derive(serde::Serialize)]
struct AdminDocsModelDetailContext {
    app_label: String,
    struct_name: String,
    table_name: String,
    ordering: String,
    fields: Vec<AdminDocsFieldRow>,
    relations: Vec<AdminDocsRelationRow>,
    indexes: Vec<AdminDocsIndexRow>,
    unique_together: Vec<String>,
    site_header: String,
    site_title: String,
    logo_url: Option<String>,
    accent_color: Option<String>,
}

/// Renders the AdminDocs index page (`GET /admin/doc/`).
pub(crate) async fn admindocs_index(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let _user = require_staff(&req).await?;

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/admindocs_index.html",
        AdminDocsIndexContext {
            model_count: registry.len(),
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}

/// Renders the AdminDocs models overview page (`GET /admin/doc/models/`).
pub(crate) async fn admindocs_models(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let _user = require_staff(&req).await?;

    let models: Vec<AdminDocsModelSummary> = registry
        .iter()
        .map(|meta| {
            let field_names = meta.fields.iter().map(|f| f.name.to_string()).collect();
            let relation_names = meta
                .relations
                .iter()
                .map(|r| {
                    let target = (r.target)();
                    format!(
                        "{} -> {}.{}",
                        r.field_name, target.app_label, target.struct_name
                    )
                })
                .collect();
            let mut constraint_descriptions: Vec<String> = meta
                .unique_together
                .iter()
                .map(|cols| format!("UNIQUE ({})", cols.join(", ")))
                .collect();
            for idx in meta.indexes {
                constraint_descriptions.push(format!(
                    "INDEX {} ({})",
                    idx.name,
                    idx.fields.join(", ")
                ));
            }
            AdminDocsModelSummary {
                app_label: meta.app_label.to_string(),
                struct_name: meta.struct_name.to_string(),
                model_slug: meta.struct_name.to_lowercase(),
                table_name: meta.table_name.to_string(),
                field_names,
                relation_names,
                constraint_descriptions,
            }
        })
        .collect();

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/admindocs_models.html",
        AdminDocsModelsContext {
            models,
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}

/// Renders the detailed AdminDocs reference page for a single registered model
/// (`GET /admin/doc/models/{app}/{model}/`).
pub(crate) async fn admindocs_model_detail(
    req: Request,
    params: PathParams,
    registry: Vec<&'static ModelMeta>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let _user = require_staff(&req).await?;

    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let meta = registry
        .iter()
        .copied()
        .find(|m| m.app_label == app && m.struct_name.to_lowercase() == model)
        .ok_or(DjangorsError::NotFound)?;

    let fields: Vec<AdminDocsFieldRow> = meta
        .fields
        .iter()
        .map(|f| AdminDocsFieldRow {
            name: f.name.to_string(),
            column_name: f.column_name.to_string(),
            kind: format!("{:?}", f.kind),
            nullable: f.nullable,
            primary_key: f.primary_key,
            unique: f.unique,
            db_index: f.db_index,
            max_length: f.max_length,
            choices: f
                .choices
                .iter()
                .map(|(v, l)| format!("{v} ({l})"))
                .collect(),
            help_text: f.help_text.unwrap_or("").to_string(),
        })
        .collect();

    let relations: Vec<AdminDocsRelationRow> = meta
        .relations
        .iter()
        .map(|r| {
            let target = (r.target)();
            AdminDocsRelationRow {
                field_name: r.field_name.to_string(),
                kind: format!("{:?}", r.kind),
                target: format!("{}.{}", target.app_label, target.struct_name),
                on_delete: format!("{:?}", r.on_delete),
                related_name: r.related_name.unwrap_or("-").to_string(),
            }
        })
        .collect();

    let indexes: Vec<AdminDocsIndexRow> = meta
        .indexes
        .iter()
        .map(|idx| AdminDocsIndexRow {
            name: idx.name.to_string(),
            fields: idx.fields.join(", "),
        })
        .collect();

    let unique_together: Vec<String> = meta
        .unique_together
        .iter()
        .map(|cols| cols.join(", "))
        .collect();

    let ordering = if meta.ordering.is_empty() {
        "-".to_string()
    } else {
        meta.ordering.join(", ")
    };

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/admindocs_model_detail.html",
        AdminDocsModelDetailContext {
            app_label: meta.app_label.to_string(),
            struct_name: meta.struct_name.to_string(),
            table_name: meta.table_name.to_string(),
            ordering,
            fields,
            relations,
            indexes,
            unique_together,
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}
