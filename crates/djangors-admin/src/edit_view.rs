use crate::forms::{build_inline_tables, render_form};
use crate::log::{log_action, FieldDiffItem, ACTION_ADDITION, ACTION_CHANGE};
use crate::model_admin::{parse_field_value, parse_relation_value};
use crate::perms::{action_codename, require_perm, resolve_tenant};
use crate::{ModelAdmin, SiteBranding, TenantScope};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use std::sync::Arc;

pub(crate) async fn admin_add_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "add").await?;
    let _tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let meta = admin.model_meta();
    let field_names = admin.field_names();
    let submitted_values = std::collections::HashMap::new();
    let errors = std::collections::HashMap::new();

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    render_form(
        meta,
        &field_names,
        &submitted_values,
        &errors,
        true,
        &branding,
        csrf_token,
        true,
        false,
        admin.fieldsets(),
        admin.excluded_fields(),
        admin.readonly_fields(),
        admin.raw_id_fields(),
        vec![],
    )
}

pub(crate) async fn admin_add_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "add").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    match admin.create_from_form(db, &form_data, tenant).await? {
        Ok(new_pk) => {
            log_action(
                db,
                user.id,
                admin.model_meta(),
                new_pk,
                ACTION_ADDITION,
                "Added.",
                None,
            )
            .await?;
            Ok(Response::redirect(&format!("/{}/{}/", app, model)))
        }
        Err(errors) => {
            let meta = admin.model_meta();
            let field_names = admin.field_names();
            let csrf_token = req
                .ext::<djangors_core::middleware::CsrfToken>()
                .map(|t| t.0.clone())
                .unwrap_or_default();
            render_form(
                meta,
                &field_names,
                &form_data,
                &errors,
                true,
                &branding,
                csrf_token,
                true,
                false,
                admin.fieldsets(),
                admin.excluded_fields(),
                admin.readonly_fields(),
                admin.raw_id_fields(),
                vec![],
            )
        }
    }
}

pub(crate) async fn admin_change_get(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let meta = admin.model_meta();
    let has_delete = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "delete");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };

    let row_opt = admin.get_by_pk(db, pk, tenant).await?;
    let row_vals = row_opt.ok_or(DjangorsError::NotFound)?;

    let mut submitted_values = std::collections::HashMap::new();
    for (name, val) in row_vals {
        submitted_values.insert(name.to_string(), val.to_string());
    }

    let field_names = admin.field_names();
    let errors = std::collections::HashMap::new();

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    let inlines_data = build_inline_tables(db, admin.inlines_config(), pk).await?;
    render_form(
        meta,
        &field_names,
        &submitted_values,
        &errors,
        false,
        &branding,
        csrf_token,
        true,
        has_delete,
        admin.fieldsets(),
        admin.excluded_fields(),
        admin.readonly_fields(),
        admin.raw_id_fields(),
        inlines_data,
    )
}

pub(crate) async fn admin_change_post(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
    branding: SiteBranding,
    tenant_scope: Option<TenantScope>,
) -> Result<Response, DjangorsError> {
    let app = params.get("app").unwrap_or("");
    let model = params.get("model").unwrap_or("");
    let pk = params
        .get_as::<i64>("pk")
        .map_err(|_| DjangorsError::BadRequest("invalid pk".to_string()))?;

    let admin = admins
        .iter()
        .find(|a| {
            let meta = a.model_meta();
            meta.app_label == app && meta.struct_name.to_lowercase() == model
        })
        .ok_or(DjangorsError::NotFound)?;

    let db = req.require_state::<djangors_db::Database>()?;

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let Form(form_data) =
        Form::<std::collections::HashMap<String, String>>::from_request(&req).await?;

    let old_row_vals = admin.get_by_pk(db, pk, tenant).await?;

    match admin.update_from_form(db, pk, &form_data, tenant).await? {
        Ok(()) => {
            let field_diff = if let Some(old_vals) = old_row_vals {
                let meta = admin.model_meta();
                let mut diff_items = Vec::new();
                for (f_name, old_val) in old_vals {
                    if admin.excluded_fields().contains(&f_name) {
                        continue;
                    }
                    let mut new_val_opt = None;
                    if let Some(f) = meta.fields.iter().find(|f| f.name == f_name) {
                        if !f.auto && !f.primary_key {
                            let raw = form_data.get(f.name).map(|s| s.as_str());
                            if let Ok(nv) = parse_field_value(f, raw) {
                                new_val_opt = Some(nv);
                            }
                        }
                    } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == f_name) {
                        let raw = form_data.get(r.field_name).map(|s| s.as_str());
                        if let Ok(nv) = parse_relation_value(r, raw) {
                            new_val_opt = Some(nv);
                        }
                    }

                    if let Some(new_val) = new_val_opt {
                        if old_val != new_val {
                            diff_items.push(FieldDiffItem {
                                field: f_name.to_string(),
                                old: old_val.to_string(),
                                new: new_val.to_string(),
                            });
                        }
                    }
                }
                if diff_items.is_empty() {
                    None
                } else {
                    serde_json::to_string(&diff_items).ok()
                }
            } else {
                None
            };

            log_action(
                db,
                user.id,
                admin.model_meta(),
                pk,
                ACTION_CHANGE,
                "Changed.",
                field_diff,
            )
            .await?;
            Ok(Response::redirect(&format!("/{}/{}/", app, model)))
        }
        Err(errors) => {
            let row_vals = admin
                .get_by_pk(db, pk, tenant)
                .await?
                .ok_or(DjangorsError::NotFound)?;

            let mut merged_form_data = form_data.clone();
            for (name, val) in row_vals {
                let meta = admin.model_meta();
                if let Some(f) = meta.fields.iter().find(|f| f.name == name) {
                    if f.auto || f.primary_key {
                        merged_form_data.insert(name.to_string(), val.to_string());
                    }
                }
            }

            let meta = admin.model_meta();
            let has_delete = if user.is_superuser {
                true
            } else {
                let codename = action_codename(meta, "delete");
                djangors_auth::has_perm(db, user.id, &codename)
                    .await
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?
            };
            let field_names = admin.field_names();
            let csrf_token = req
                .ext::<djangors_core::middleware::CsrfToken>()
                .map(|t| t.0.clone())
                .unwrap_or_default();
            let inlines_data = build_inline_tables(db, admin.inlines_config(), pk).await?;
            render_form(
                meta,
                &field_names,
                &merged_form_data,
                &errors,
                false,
                &branding,
                csrf_token,
                true,
                has_delete,
                admin.fieldsets(),
                admin.excluded_fields(),
                admin.readonly_fields(),
                admin.raw_id_fields(),
                inlines_data,
            )
        }
    }
}
