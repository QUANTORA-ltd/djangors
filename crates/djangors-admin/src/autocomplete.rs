use crate::perms::{require_perm, resolve_tenant};
use crate::{ModelAdmin, TenantScope};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_orm::meta::{FieldKind, ModelMeta};
use std::sync::Arc;

const AUTOCOMPLETE_PAGE_SIZE: usize = 20;

#[derive(serde::Serialize)]
struct AutocompleteItem {
    id: String,
    text: String,
}

#[derive(serde::Serialize)]
struct AutocompletePagination {
    more: bool,
}

#[derive(serde::Serialize)]
struct AutocompleteResponse {
    results: Vec<AutocompleteItem>,
    pagination: AutocompletePagination,
}

fn find_pk_column(meta: &'static ModelMeta) -> &'static str {
    meta.fields
        .iter()
        .find(|f| f.primary_key)
        .map(|f| f.column_name)
        .unwrap_or("id")
}

fn find_display_column(meta: &'static ModelMeta, requested_field: Option<&str>) -> &'static str {
    if let Some(req_f) = requested_field {
        if let Some(f) = meta.fields.iter().find(|f| f.name == req_f) {
            return f.column_name;
        }
    }
    meta.fields
        .iter()
        .find(|f| {
            !f.primary_key
                && matches!(
                    f.kind,
                    FieldKind::Char
                        | FieldKind::Text
                        | FieldKind::Slug
                        | FieldKind::Email
                        | FieldKind::Url
                )
        })
        .map(|f| f.column_name)
        .unwrap_or_else(|| find_pk_column(meta))
}

pub(crate) async fn admin_autocomplete(
    req: Request,
    params: PathParams,
    admins: Vec<Arc<dyn ModelAdmin>>,
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
    let user = require_perm(&req, db, admin.model_meta(), "view").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let term = req.query("term").unwrap_or("").trim();
    let field_param = req.query("field");

    let meta = admin.model_meta();
    // If `field` names a relation on this model, query the target model's table.
    // Otherwise query this model's table directly.
    let (target_meta, is_relation_target) = match field_param {
        Some(f) => {
            if let Some(rel) = meta.relations.iter().find(|r| r.field_name == f) {
                ((rel.target)(), true)
            } else {
                (meta, false)
            }
        }
        None => (meta, false),
    };

    let pk_col = find_pk_column(target_meta);
    let display_col = find_display_column(
        target_meta,
        if is_relation_target {
            None
        } else {
            field_param
        },
    );

    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk_quoted = dialect.quote_ident(pk_col);
    let display_quoted = dialect.quote_ident(display_col);
    let table_quoted = dialect.quote_ident(target_meta.table_name);

    let mut where_clauses: Vec<String> = Vec::new();
    let mut binds: Vec<djangors_db::BindValue> = Vec::new();

    if !term.is_empty() {
        let p = dialect.placeholder(binds.len() + 1);
        where_clauses.push(format!(
            "LOWER(CAST({display_quoted} AS TEXT)) LIKE LOWER({p})"
        ));
        binds.push(djangors_db::BindValue::Text(format!("%{term}%")));
    }

    // Apply tenant restriction if querying the same model (or target has the tenant column).
    if let Some((tenant_field, tenant_id)) = tenant {
        let has_tenant_col = target_meta.fields.iter().any(|f| f.name == tenant_field)
            || target_meta
                .relations
                .iter()
                .any(|r| r.field_name == tenant_field);
        if has_tenant_col {
            let p = dialect.placeholder(binds.len() + 1);
            let t_quoted = dialect.quote_ident(tenant_field);
            where_clauses.push(format!("{t_quoted} = {p}"));
            binds.push(djangors_db::BindValue::I64(tenant_id));
        }
    }

    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", where_clauses.join(" AND "))
    };

    let limit = AUTOCOMPLETE_PAGE_SIZE + 1;
    let sql = format!(
        "SELECT CAST({pk_quoted} AS TEXT) AS pk_val, CAST({display_quoted} AS TEXT) AS text_val \
         FROM {table_quoted}{where_sql} ORDER BY {pk_quoted} ASC LIMIT {limit}"
    );

    let rows = conn
        .fetch_all(&sql, &binds)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let more = rows.len() > AUTOCOMPLETE_PAGE_SIZE;
    let results: Vec<AutocompleteItem> = rows
        .into_iter()
        .take(AUTOCOMPLETE_PAGE_SIZE)
        .map(|row| {
            let id = row.try_string(0).ok().flatten().unwrap_or_default();
            let text = row
                .try_string(1)
                .ok()
                .flatten()
                .unwrap_or_else(|| id.clone());
            AutocompleteItem { id, text }
        })
        .collect();

    let payload = AutocompleteResponse {
        results,
        pagination: AutocompletePagination { more },
    };
    let json = serde_json::to_vec(&payload).map_err(|e| DjangorsError::Internal(e.to_string()))?;

    Ok(Response::bytes(StatusCode::OK, "application/json", json))
}
