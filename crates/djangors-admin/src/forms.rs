use crate::{InlineConfig, SiteBranding, ADMIN_TEMPLATES};
use djangors_core::{DjangorsError, Response};
use djangors_orm::meta::ModelMeta;

#[derive(serde::Serialize)]
pub(crate) struct FormFieldRow {
    pub(crate) kind: &'static str, // "readonly" | "checkbox" | "number" | "text"
    pub(crate) name: String,
    pub(crate) value: String, // readonly span text, or the input's value="" attribute; unused for checkbox
    pub(crate) checked: bool, // only meaningful when kind == "checkbox"
    pub(crate) error: Option<String>,
    pub(crate) section: Option<&'static str>,
    pub(crate) lookup_href: Option<minijinja::value::Value>,
}

#[derive(serde::Serialize)]
pub(crate) struct InlineRowData {
    pub(crate) values: Vec<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct InlineTableData {
    pub(crate) struct_name: String,
    pub(crate) fields: Vec<String>,
    pub(crate) rows: Vec<InlineRowData>,
}

#[derive(serde::Serialize)]
pub(crate) struct RenderFormContext {
    pub(crate) rows: Vec<FormFieldRow>,
    pub(crate) inlines: Vec<InlineTableData>,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) csrf_token: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
    pub(crate) has_change_permission: bool,
    pub(crate) has_delete_permission: bool,
    pub(crate) is_add: bool,
}

pub(crate) async fn build_inline_tables(
    db: &djangors_db::Database,
    inlines: &[InlineConfig],
    pk: i64,
) -> Result<Vec<InlineTableData>, DjangorsError> {
    let mut tables = Vec::new();
    for ic in inlines {
        let child_meta = djangors_orm::meta::all_registered_models()
            .find(|m| m.struct_name == ic.struct_name)
            .ok_or_else(|| DjangorsError::Internal(format!("unknown model {}", ic.struct_name)))?;

        let mut conn = db.conn();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let cols: Vec<String> = ic.fields.iter().map(|f| format!("\"{}\"", f)).collect();
        let cols_str = cols.join(", ");
        let sql = format!(
            "SELECT {cols_str} FROM \"{}\" WHERE \"{}\" = {p1}",
            child_meta.table_name, ic.relation_field
        );
        let rows = conn
            .fetch_all(&sql, &[djangors_db::BindValue::I64(pk)])
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let mut data_rows = Vec::new();
        for row in &rows {
            let mut values = Vec::new();
            for f in ic.fields {
                let val = row
                    .try_string_by_name(f)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "-".to_string());
                values.push(val);
            }
            data_rows.push(InlineRowData { values });
        }

        tables.push(InlineTableData {
            struct_name: ic.struct_name.to_string(),
            fields: ic.fields.iter().map(|s| s.to_string()).collect(),
            rows: data_rows,
        });
    }
    Ok(tables)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_form(
    meta: &'static ModelMeta,
    field_names: &[&'static str],
    submitted_values: &std::collections::HashMap<String, String>,
    errors: &std::collections::HashMap<String, String>,
    is_add: bool,
    branding: &SiteBranding,
    csrf_token: String,
    has_change_permission: bool,
    has_delete_permission: bool,
    fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    excluded_fields: &[&'static str],
    readonly_fields: &[&'static str],
    raw_id_fields: &[&'static str],
    inlines_data: Vec<InlineTableData>,
) -> Result<Response, DjangorsError> {
    let build_row = |name: &'static str, section: Option<&'static str>| -> Option<FormFieldRow> {
        if excluded_fields.contains(&name) {
            return None;
        }
        if let Some(field) = meta.fields.iter().find(|f| f.name == name) {
            if field.auto || field.primary_key {
                if is_add {
                    return None;
                } else {
                    let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
                    return Some(FormFieldRow {
                        kind: "readonly",
                        name: name.to_string(),
                        value: val.to_string(),
                        checked: false,
                        error: None,
                        section,
                        lookup_href: None,
                    });
                }
            }

            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let err = errors.get(name).cloned();

            let is_readonly = readonly_fields.contains(&name);
            let kind = if is_readonly {
                "readonly"
            } else {
                match field.kind {
                    djangors_orm::meta::FieldKind::Boolean => "checkbox",
                    djangors_orm::meta::FieldKind::Integer
                    | djangors_orm::meta::FieldKind::BigInt
                    | djangors_orm::meta::FieldKind::Float => "number",
                    _ => "text",
                }
            };

            let checked = kind == "checkbox" && (val == "on" || val == "true");

            Some(FormFieldRow {
                kind,
                name: name.to_string(),
                value: val.to_string(),
                checked,
                error: err,
                section,
                lookup_href: None,
            })
        } else if let Some(rel) = meta.relations.iter().find(|r| r.field_name == name) {
            let val = submitted_values.get(name).map(|s| s.as_str()).unwrap_or("");
            let err = errors.get(name).cloned();

            let is_readonly = readonly_fields.contains(&name);
            let kind = if is_readonly { "readonly" } else { "number" };

            let lookup_href = if raw_id_fields.contains(&name) {
                let target_meta = (rel.target)();
                Some(minijinja::value::Value::from_safe_string(format!(
                    "/{}/{}/",
                    target_meta.app_label,
                    target_meta.struct_name.to_lowercase()
                )))
            } else {
                None
            };

            Some(FormFieldRow {
                kind,
                name: name.to_string(),
                value: val.to_string(),
                checked: false,
                error: err,
                section,
                lookup_href,
            })
        } else {
            None
        }
    };

    let mut rows = Vec::new();

    if let Some(fsets) = fieldsets {
        for &(section_title, names) in fsets {
            for &name in names {
                if let Some(row) = build_row(name, Some(section_title)) {
                    rows.push(row);
                }
            }
        }
        // Also add any field names not in any fieldset
        let all_fieldset_names: std::collections::HashSet<&'static str> = fsets
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();
        for &name in field_names {
            if !all_fieldset_names.contains(name) {
                if let Some(row) = build_row(name, None) {
                    rows.push(row);
                }
            }
        }
    } else {
        for &name in field_names {
            if let Some(row) = build_row(name, None) {
                rows.push(row);
            }
        }
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/render_form.html",
        RenderFormContext {
            rows,
            inlines: inlines_data,
            site_header: branding.site_header.clone(),
            site_title: branding.site_title.clone(),
            csrf_token,
            logo_url: branding.logo_url.clone(),
            accent_color: branding.accent_color.clone(),
            has_change_permission,
            has_delete_permission,
            is_add,
        },
    )
}
