use crate::{InlineConfig, ModelAdmin, SiteBranding, ADMIN_TEMPLATES};
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
    pub(crate) is_autocomplete: bool,
    pub(crate) is_filter_horizontal: bool,
    pub(crate) is_filter_vertical: bool,
}

#[derive(serde::Serialize)]
pub(crate) struct PrepopulatedFieldData {
    pub(crate) target: String,
    pub(crate) sources: Vec<String>,
}

pub(crate) struct FormOptions<'a> {
    pub(crate) fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    pub(crate) readonly_fields: &'a [&'static str],
    pub(crate) raw_id_fields: &'a [&'static str],
    pub(crate) prepopulated_fields: &'a [(&'static str, &'static [&'static str])],
    pub(crate) autocomplete_fields: &'a [&'static str],
    pub(crate) filter_horizontal: &'a [&'static str],
    pub(crate) filter_vertical: &'a [&'static str],
    pub(crate) save_as: bool,
    pub(crate) save_on_top: bool,
}

impl<'a> FormOptions<'a> {
    pub(crate) fn from_admin(admin: &'a dyn ModelAdmin) -> Self {
        Self {
            fieldsets: admin.fieldsets(),
            readonly_fields: admin.readonly_fields(),
            raw_id_fields: admin.raw_id_fields(),
            prepopulated_fields: admin.prepopulated_fields(),
            autocomplete_fields: admin.autocomplete_fields(),
            filter_horizontal: admin.filter_horizontal(),
            filter_vertical: admin.filter_vertical(),
            save_as: admin.save_as(),
            save_on_top: admin.save_on_top(),
        }
    }
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
    pub(crate) prepopulated_fields: Vec<PrepopulatedFieldData>,
    pub(crate) autocomplete_fields: Vec<String>,
    pub(crate) filter_horizontal: Vec<String>,
    pub(crate) filter_vertical: Vec<String>,
    pub(crate) save_as: bool,
    pub(crate) save_on_top: bool,
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
    options: &FormOptions<'_>,
    inlines_data: Vec<InlineTableData>,
) -> Result<Response, DjangorsError> {
    let fieldsets = options.fieldsets;
    let readonly_fields = options.readonly_fields;
    let raw_id_fields = options.raw_id_fields;
    let autocomplete_fields = options.autocomplete_fields;
    let filter_horizontal = options.filter_horizontal;
    let filter_vertical = options.filter_vertical;

    let build_row = |name: &'static str, section: Option<&'static str>| -> Option<FormFieldRow> {
        let is_autocomplete = autocomplete_fields.contains(&name);
        let is_filter_horizontal = filter_horizontal.contains(&name);
        let is_filter_vertical = filter_vertical.contains(&name);

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
                        is_autocomplete: false,
                        is_filter_horizontal: false,
                        is_filter_vertical: false,
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
                is_autocomplete,
                is_filter_horizontal,
                is_filter_vertical,
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
                is_autocomplete,
                is_filter_horizontal,
                is_filter_vertical,
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

    let prepopulated_fields: Vec<PrepopulatedFieldData> = options
        .prepopulated_fields
        .iter()
        .map(|(target, sources)| PrepopulatedFieldData {
            target: (*target).to_string(),
            sources: sources.iter().map(|s| (*s).to_string()).collect(),
        })
        .collect();

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
            prepopulated_fields,
            autocomplete_fields: autocomplete_fields
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            filter_horizontal: filter_horizontal.iter().map(|s| (*s).to_string()).collect(),
            filter_vertical: filter_vertical.iter().map(|s| (*s).to_string()).collect(),
            save_as: options.save_as,
            save_on_top: options.save_on_top,
        },
    )
}
