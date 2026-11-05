use crate::log::{log_action, AdminActionRow, ACTION_CHANGE};
use crate::perms::{action_codename, require_perm, resolve_tenant};
use crate::{ModelAdmin, SiteBranding, TenantScope, ADMIN_TEMPLATES, CHANGELIST_PER_PAGE};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use std::sync::Arc;

/// Percent-encodes a value for embedding in a `href="?...&key=<value>"` query
/// string. HTML-escaping alone is not enough here: `&` in a raw search term
/// would be rendered as `&amp;`, which a browser decodes right back to a
/// literal `&` before parsing the URL, letting the value inject extra query
/// parameters or truncate the link (via `#`) instead of round-tripping as a
/// single opaque value.
pub(crate) fn url_encode_query_value(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Builds a `?key=value&key=value...` query string from already-string-ish
/// parts, percent-encoding every value. Skips pairs whose value is `None`.
pub(crate) fn build_query_string(pairs: &[(&str, Option<&str>)]) -> String {
    let parts: Vec<String> = pairs
        .iter()
        .filter_map(|(k, v)| v.map(|val| format!("{}={}", k, url_encode_query_value(val))))
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

pub(crate) fn csv_escape_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub(crate) fn rows_to_csv(columns: &[&'static str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push_str(
        &columns
            .iter()
            .map(|c| csv_escape_field(c))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push_str("\r\n");
    for row in rows {
        out.push_str(
            &row.iter()
                .map(|v| csv_escape_field(v))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    out
}

pub(crate) async fn admin_export_csv(
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

    let state = parse_changelist_query_state(&req, admin.as_ref())?;

    let (columns, rows) = admin
        .export_csv_rows(
            db,
            state.order,
            state.search,
            &state.filters,
            state.date_range,
            tenant,
        )
        .await?;
    let csv_body = rows_to_csv(&columns, &rows);

    let filename = format!("{}.csv", admin.model_meta().struct_name.to_lowercase());
    Ok(Response::bytes(
        StatusCode::OK,
        "text/csv; charset=utf-8",
        csv_body.into_bytes(),
    )
    .header(
        "Content-Disposition",
        &format!("attachment; filename=\"{}\"", filename),
    ))
}

pub(crate) fn get_month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

// Holds the parsed query-state (ordering, search, filter flags, and date-hierarchy parameters)
// used to filter and paginate model instances displayed on the admin changelist page.
pub(crate) struct ChangelistQueryState<'a> {
    // Sorting order parameter parsed from ?o= (e.g. "field" or "-field").
    pub(crate) order: Option<&'a str>,
    // Search term parsed from ?q=.
    pub(crate) search: Option<&'a str>,
    // Active boolean filter flags parsed from query fields.
    pub(crate) filters: Vec<(&'static str, bool)>,
    // Optional year filter for date-hierarchy navigation.
    pub(crate) year: Option<i32>,
    // Optional month filter for date-hierarchy navigation.
    pub(crate) month: Option<u32>,
    // Optional day filter for date-hierarchy navigation.
    pub(crate) day: Option<u32>,
    // Derived UTC timestamp range matching the hierarchical date filter.
    pub(crate) date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
}

// Parses URL query parameters (ordering, search query, active filters, and date hierarchical
// parameters) from the incoming request into `ChangelistQueryState`.
pub(crate) fn parse_changelist_query_state<'a>(
    req: &'a Request,
    admin: &dyn ModelAdmin,
) -> Result<ChangelistQueryState<'a>, DjangorsError> {
    let order = req.query("o");
    let search = req.query("q");

    let year: Option<i32> = match req.query("year") {
        Some(s) => Some(
            s.parse()
                .map_err(|_| DjangorsError::BadRequest("invalid year".into()))?,
        ),
        None => None,
    };
    let month: Option<u32> = match req.query("month") {
        Some(s) => Some(
            s.parse()
                .map_err(|_| DjangorsError::BadRequest("invalid month".into()))?,
        ),
        None => None,
    };
    let day: Option<u32> = match req.query("day") {
        Some(s) => Some(
            s.parse()
                .map_err(|_| DjangorsError::BadRequest("invalid day".into()))?,
        ),
        None => None,
    };

    use chrono::{Duration, TimeZone, Utc};

    let date_range: Option<(chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
        match (admin.date_hierarchy_field(), year) {
            (Some(_), Some(y)) => {
                let (gte, lt) = match month {
                    Some(m) => {
                        let start =
                            Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0)
                                .single()
                                .ok_or_else(|| {
                                    DjangorsError::BadRequest("invalid year/month".into())
                                })?;
                        let end = match day {
                            Some(d) => {
                                let s =
                                    Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).single().ok_or_else(
                                        || DjangorsError::BadRequest("invalid day".into()),
                                    )?;
                                s + Duration::days(1)
                            }
                            None => if m == 12 {
                                Utc.with_ymd_and_hms(y + 1, 1, 1, 0, 0, 0).single()
                            } else {
                                Utc.with_ymd_and_hms(y, m + 1, 1, 0, 0, 0).single()
                            }
                            .ok_or_else(|| {
                                DjangorsError::BadRequest("invalid year/month".into())
                            })?,
                        };
                        let start = match day {
                            Some(d) => Utc
                                .with_ymd_and_hms(y, m, d, 0, 0, 0)
                                .single()
                                .ok_or_else(|| DjangorsError::BadRequest("invalid day".into()))?,
                            None => start,
                        };
                        (start, end)
                    }
                    None => {
                        let start = Utc
                            .with_ymd_and_hms(y, 1, 1, 0, 0, 0)
                            .single()
                            .ok_or_else(|| DjangorsError::BadRequest("invalid year".into()))?;
                        let end = Utc
                            .with_ymd_and_hms(y + 1, 1, 1, 0, 0, 0)
                            .single()
                            .ok_or_else(|| DjangorsError::BadRequest("invalid year".into()))?;
                        (start, end)
                    }
                };
                Some((gte, lt))
            }
            _ => None,
        };

    let list_filter_fields = admin.list_filter_fields();
    let mut active_filters: Vec<(&'static str, bool)> = Vec::new();
    for field_name in list_filter_fields {
        if let Some(raw) = req.query(field_name) {
            match raw {
                "true" => active_filters.push((field_name, true)),
                "false" => active_filters.push((field_name, false)),
                _ => {} // anything else (including empty/"all") = not filtered on this field
            }
        }
    }

    Ok(ChangelistQueryState {
        order,
        search,
        filters: active_filters,
        year,
        month,
        day,
        date_range,
    })
}

// As per design, all href, value, and label fields in the changelist context are treated
// as plain String/Option<String> rather than Safe-wrapped minijinja::value::Value.
// This is because build_query_string returns raw URLs with unescaped ampersands.
// Entity-escaping via autoescape correctly turns '&' into '&amp;', which is spec-compliant
// and doesn't break any test assertions.
#[derive(serde::Serialize)]
pub(crate) struct HeaderCellData {
    pub(crate) href: String,
    pub(crate) label: String,
}

#[derive(serde::Serialize)]
pub(crate) struct ChangelistCellData {
    pub(crate) kind: &'static str, // "pk_link" | "editable" | "plain"
    pub(crate) value: String,
    pub(crate) field_name: String, // only meaningful when kind == "editable"
}

#[derive(serde::Serialize)]
pub(crate) struct ChangelistRowData {
    pub(crate) pk: String,
    pub(crate) cells: Vec<ChangelistCellData>,
}

#[derive(serde::Serialize)]
pub(crate) struct PagerData {
    pub(crate) prev_href: Option<String>,
    pub(crate) page: i64,
    pub(crate) total_pages: i64,
    pub(crate) total: i64,
    pub(crate) next_href: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct HiddenInputData {
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(serde::Serialize)]
pub(crate) struct SearchBoxData {
    pub(crate) visible: bool,
    pub(crate) hidden_inputs: Vec<HiddenInputData>,
    pub(crate) q_value: String,
}

#[derive(serde::Serialize)]
pub(crate) struct FilterBlockData {
    pub(crate) field: String,
    pub(crate) all_href: String,
    pub(crate) yes_href: String,
    pub(crate) no_href: String,
}

#[derive(serde::Serialize)]
pub(crate) struct DateHierarchyLinkData {
    pub(crate) href: String,
    pub(crate) label: String,
}

#[derive(serde::Serialize)]
pub(crate) struct DateHierarchyData {
    pub(crate) visible: bool,
    pub(crate) breadcrumbs: Vec<DateHierarchyLinkData>,
    pub(crate) links: Vec<DateHierarchyLinkData>,
}

#[derive(serde::Serialize)]
pub(crate) struct ChangelistTemplateContext {
    pub(crate) date_hierarchy: DateHierarchyData,
    pub(crate) search: SearchBoxData,
    pub(crate) list_filter_blocks: Vec<FilterBlockData>,
    pub(crate) actions: Vec<AdminActionRow>,
    pub(crate) header_cells: Vec<HeaderCellData>,
    pub(crate) rows: Vec<ChangelistRowData>,
    pub(crate) show_save_button: bool,
    pub(crate) pager: PagerData,
    pub(crate) export_query: String,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) csrf_token: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
    pub(crate) has_add_permission: bool,
    pub(crate) has_change_permission: bool,
    pub(crate) has_delete_permission: bool,
}

pub(crate) async fn admin_changelist(
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

    let user = require_perm(&req, db, admin.model_meta(), "view").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let meta = admin.model_meta();
    let has_add = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "add");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };
    let has_change = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "change");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };
    let has_delete = if user.is_superuser {
        true
    } else {
        let codename = action_codename(meta, "delete");
        djangors_auth::has_perm(db, user.id, &codename)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
    };

    let admin_actions = admin.actions();
    let actions_rows: Vec<AdminActionRow> = admin_actions
        .iter()
        .map(|a| AdminActionRow {
            name: a.name.to_string(),
            label: a.label.to_string(),
        })
        .collect();

    let state = parse_changelist_query_state(&req, admin.as_ref())?;
    let o = state.order;
    let q = state.search;
    let active_filters = state.filters;
    let year = state.year;
    let month = state.month;
    let day = state.day;
    let date_range = state.date_range;

    let year_str = year.map(|y| y.to_string());
    let month_str = month.map(|m| m.to_string());
    let day_str = day.map(|d| d.to_string());

    let list_filter_fields = admin.list_filter_fields();

    let page = match req.query("page") {
        Some(p_str) => {
            let p = p_str
                .parse::<i64>()
                .map_err(|_| DjangorsError::BadRequest("invalid page parameter".to_string()))?;
            if p < 1 {
                return Err(DjangorsError::BadRequest("page must be >= 1".to_string()));
            }
            p
        }
        None => 1,
    };

    let page_data = admin
        .changelist(
            db,
            o,
            page,
            CHANGELIST_PER_PAGE,
            q,
            &active_filters,
            date_range,
            tenant,
        )
        .await?;

    let mut header_cells = Vec::new();
    for col in &page_data.columns {
        let new_o = if o == Some(*col) {
            format!("-{}", col)
        } else {
            col.to_string()
        };
        let mut pairs = vec![("o", Some(new_o.as_str())), ("q", q)];
        for &(f, val) in &active_filters {
            pairs.push((f, Some(if val { "true" } else { "false" })));
        }
        pairs.push(("year", year_str.as_deref()));
        pairs.push(("month", month_str.as_deref()));
        pairs.push(("day", day_str.as_deref()));

        let link = build_query_string(&pairs);
        header_cells.push(HeaderCellData {
            href: link,
            label: col.to_string(),
        });
    }

    let editable_fields = admin.list_editable_fields();
    let mut rows = Vec::new();
    for (row_index, row) in page_data.rows.into_iter().enumerate() {
        let pk = page_data.pks.get(row_index).cloned().unwrap_or_default();
        let mut cells = Vec::new();
        for (i, cell) in row.into_iter().enumerate() {
            let col_name = page_data.columns.get(i).copied().unwrap_or("");
            // Note: We escape cell values here in Rust using `djangors_core::html_escape`
            // rather than letting minijinja autoescape them. This is because `html_escape`
            // escapes `/` as `&#x2F;` (uppercase hex), whereas minijinja escapes it as `&#x2f;`
            // (lowercase hex). The integration tests verify the exact uppercase hex representation
            // for XSS checks (e.g. testing `</script>`), and would fail if lowercase was emitted.
            // Correspondingly, the template uses `|safe` to render this pre-escaped value.
            let escaped_value = djangors_core::html_escape(&cell);
            if i == 0 {
                cells.push(ChangelistCellData {
                    kind: "pk_link",
                    value: escaped_value,
                    field_name: String::new(),
                });
            } else if editable_fields.contains(&col_name) {
                cells.push(ChangelistCellData {
                    kind: "editable",
                    value: escaped_value,
                    field_name: col_name.to_string(),
                });
            } else {
                cells.push(ChangelistCellData {
                    kind: "plain",
                    value: escaped_value,
                    field_name: String::new(),
                });
            }
        }
        rows.push(ChangelistRowData { pk, cells });
    }

    let paginator = djangors_core::Paginator::new(page_data.total, CHANGELIST_PER_PAGE);
    let total_pages = paginator.total_pages();

    let prev_href = if paginator.has_previous(page) {
        let prev_page_str = (page - 1).to_string();
        let mut pairs = vec![("page", Some(prev_page_str.as_str())), ("o", o), ("q", q)];
        for &(f, val) in &active_filters {
            pairs.push((f, Some(if val { "true" } else { "false" })));
        }
        pairs.push(("year", year_str.as_deref()));
        pairs.push(("month", month_str.as_deref()));
        pairs.push(("day", day_str.as_deref()));
        Some(build_query_string(&pairs))
    } else {
        None
    };

    let next_href = if paginator.has_next(page) {
        let next_page_str = (page + 1).to_string();
        let mut pairs = vec![("page", Some(next_page_str.as_str())), ("o", o), ("q", q)];
        for &(f, val) in &active_filters {
            pairs.push((f, Some(if val { "true" } else { "false" })));
        }
        pairs.push(("year", year_str.as_deref()));
        pairs.push(("month", month_str.as_deref()));
        pairs.push(("day", day_str.as_deref()));
        Some(build_query_string(&pairs))
    } else {
        None
    };

    let pager = PagerData {
        prev_href,
        page,
        total_pages,
        total: page_data.total,
        next_href,
    };

    let search_fields = admin.search_fields();
    let search_visible = !search_fields.is_empty();
    let mut hidden_inputs = Vec::new();
    if search_visible {
        if let Some(order_val) = o {
            hidden_inputs.push(HiddenInputData {
                name: "o".to_string(),
                value: order_val.to_string(),
            });
        }
        for &(f, val) in &active_filters {
            hidden_inputs.push(HiddenInputData {
                name: f.to_string(),
                value: (if val { "true" } else { "false" }).to_string(),
            });
        }
        if let Some(ref y) = year_str {
            hidden_inputs.push(HiddenInputData {
                name: "year".to_string(),
                value: y.clone(),
            });
        }
        if let Some(ref m) = month_str {
            hidden_inputs.push(HiddenInputData {
                name: "month".to_string(),
                value: m.clone(),
            });
        }
        if let Some(ref d) = day_str {
            hidden_inputs.push(HiddenInputData {
                name: "day".to_string(),
                value: d.clone(),
            });
        }
    }
    let search = SearchBoxData {
        visible: search_visible,
        hidden_inputs,
        // The query string value is also pre-escaped with `djangors_core::html_escape` to preserve
        // uppercase hex `/` escaping in HTML assertions, matching the cell value escaping strategy.
        q_value: q.map(djangors_core::html_escape).unwrap_or_default(),
    };

    let mut list_filter_blocks = Vec::new();
    if !list_filter_fields.is_empty() {
        for filter_field in list_filter_fields {
            let mut pairs_all = vec![("o", o), ("q", q)];
            for &(f, val) in &active_filters {
                if f != filter_field {
                    pairs_all.push((f, Some(if val { "true" } else { "false" })));
                }
            }
            pairs_all.push(("year", year_str.as_deref()));
            pairs_all.push(("month", month_str.as_deref()));
            pairs_all.push(("day", day_str.as_deref()));
            let all_href = build_query_string(&pairs_all);

            let mut pairs_yes = vec![("o", o), ("q", q), (filter_field, Some("true"))];
            for &(f, val) in &active_filters {
                if f != filter_field {
                    pairs_yes.push((f, Some(if val { "true" } else { "false" })));
                }
            }
            pairs_yes.push(("year", year_str.as_deref()));
            pairs_yes.push(("month", month_str.as_deref()));
            pairs_yes.push(("day", day_str.as_deref()));
            let yes_href = build_query_string(&pairs_yes);

            let mut pairs_no = vec![("o", o), ("q", q), (filter_field, Some("false"))];
            for &(f, val) in &active_filters {
                if f != filter_field {
                    pairs_no.push((f, Some(if val { "true" } else { "false" })));
                }
            }
            pairs_no.push(("year", year_str.as_deref()));
            pairs_no.push(("month", month_str.as_deref()));
            pairs_no.push(("day", day_str.as_deref()));
            let no_href = build_query_string(&pairs_no);

            list_filter_blocks.push(FilterBlockData {
                field: filter_field.to_string(),
                all_href,
                yes_href,
                no_href,
            });
        }
    }

    let mut date_hierarchy_visible = false;
    let mut date_hierarchy_breadcrumbs = Vec::new();
    let mut date_hierarchy_links = Vec::new();

    if let Some(field) = admin.date_hierarchy_field() {
        date_hierarchy_visible = true;
        let values =
            date_hierarchy_drilldown_values(db, admin.model_meta().table_name, field, year, month)
                .await?;

        let mut pairs_all = vec![("o", o), ("q", q)];
        for &(f, val) in &active_filters {
            pairs_all.push((f, Some(if val { "true" } else { "false" })));
        }
        let all_link = build_query_string(&pairs_all);

        let mut pairs_y = vec![("o", o), ("q", q), ("year", year_str.as_deref())];
        for &(f, val) in &active_filters {
            pairs_y.push((f, Some(if val { "true" } else { "false" })));
        }
        let year_link = build_query_string(&pairs_y);

        let mut pairs_m = vec![
            ("o", o),
            ("q", q),
            ("year", year_str.as_deref()),
            ("month", month_str.as_deref()),
        ];
        for &(f, val) in &active_filters {
            pairs_m.push((f, Some(if val { "true" } else { "false" })));
        }
        let month_link = build_query_string(&pairs_m);

        if let Some(y) = year {
            if let Some(m) = month {
                let m_name = get_month_name(m);
                if let Some(d) = day {
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: all_link,
                        label: y.to_string(),
                    });
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: year_link,
                        label: m_name.to_string(),
                    });
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: month_link,
                        label: d.to_string(),
                    });
                } else {
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: all_link,
                        label: y.to_string(),
                    });
                    date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                        href: year_link,
                        label: m_name.to_string(),
                    });
                }
            } else {
                date_hierarchy_breadcrumbs.push(DateHierarchyLinkData {
                    href: all_link,
                    label: y.to_string(),
                });
            }
        }

        if day.is_none() {
            for val in values {
                let val_str = val.to_string();
                let mut pairs_val = vec![("o", o), ("q", q)];
                for &(f, val) in &active_filters {
                    pairs_val.push((f, Some(if val { "true" } else { "false" })));
                }

                let link_text = if year.is_none() {
                    pairs_val.push(("year", Some(val_str.as_str())));
                    val_str.clone()
                } else if month.is_none() {
                    pairs_val.push(("year", year_str.as_deref()));
                    pairs_val.push(("month", Some(val_str.as_str())));
                    get_month_name(val as u32).to_string()
                } else {
                    pairs_val.push(("year", year_str.as_deref()));
                    pairs_val.push(("month", month_str.as_deref()));
                    pairs_val.push(("day", Some(val_str.as_str())));
                    val_str.clone()
                };

                let val_link = build_query_string(&pairs_val);
                date_hierarchy_links.push(DateHierarchyLinkData {
                    href: val_link,
                    label: link_text,
                });
            }
        }
    }

    let date_hierarchy = DateHierarchyData {
        visible: date_hierarchy_visible,
        breadcrumbs: date_hierarchy_breadcrumbs,
        links: date_hierarchy_links,
    };

    let show_save_button = !editable_fields.is_empty();

    let mut export_pairs = vec![("o", o), ("q", q)];
    for &(f, val) in &active_filters {
        export_pairs.push((f, Some(if val { "true" } else { "false" })));
    }
    export_pairs.push(("year", year_str.as_deref()));
    export_pairs.push(("month", month_str.as_deref()));
    export_pairs.push(("day", day_str.as_deref()));
    let export_query = build_query_string(&export_pairs);

    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .unwrap_or_default();
    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/changelist.html",
        ChangelistTemplateContext {
            date_hierarchy,
            search,
            list_filter_blocks,
            header_cells,
            rows,
            show_save_button,
            pager,
            export_query,
            site_header: branding.site_header,
            site_title: branding.site_title,
            csrf_token,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
            actions: actions_rows,
            has_add_permission: has_add,
            has_change_permission: has_change,
            has_delete_permission: has_delete,
        },
    )
}

/// Returns the distinct values one drilldown level below the given
/// `(year, month)` state, in ascending order: years if `year` is `None`,
/// months within that year if `month` is `None`, else days within that
/// year+month. Computed from the whole table on `col` alone — see the design
/// doc's scope note on why this isn't combined with the current
/// search/list_filter state.
pub(crate) async fn date_hierarchy_drilldown_values(
    db: &djangors_db::Database,
    table_name: &'static str,
    col: &'static str,
    year: Option<i32>,
    month: Option<u32>,
) -> Result<Vec<i32>, DjangorsError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let year_expr = dialect.extract_date_part(djangors_db::DatePart::Year, col);
    let month_expr = dialect.extract_date_part(djangors_db::DatePart::Month, col);
    let day_expr = dialect.extract_date_part(djangors_db::DatePart::Day, col);

    let mut params = Vec::new();
    let sql = match (year, month) {
        (None, _) => format!(
            "SELECT DISTINCT {year_expr} AS v FROM {table_name} WHERE {col} IS NOT NULL ORDER BY 1"
        ),
        (Some(y), None) => {
            let p1 = dialect.placeholder(1);
            params.push(djangors_db::BindValue::I64(y as i64));
            format!(
                "SELECT DISTINCT {month_expr} AS v FROM {table_name} WHERE {col} IS NOT NULL AND {year_expr} = {p1} ORDER BY 1"
            )
        }
        (Some(y), Some(m)) => {
            let p1 = dialect.placeholder(1);
            let p2 = dialect.placeholder(2);
            params.push(djangors_db::BindValue::I64(y as i64));
            params.push(djangors_db::BindValue::I64(m as i64));
            format!(
                "SELECT DISTINCT {day_expr} AS v FROM {table_name} WHERE {col} IS NOT NULL AND {year_expr} = {p1} AND {month_expr} = {p2} ORDER BY 1"
            )
        }
    };
    let rows = conn
        .fetch_all(&sql, &params)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let mut result = Vec::new();
    for row in rows {
        if let Some(val) = row
            .try_i64(0)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
        {
            result.push(val as i32);
        }
    }
    Ok(result)
}

#[derive(serde::Serialize)]
pub(crate) struct SaveChangelistErrorRow {
    pub(crate) pk: i64,
    pub(crate) field: String,
    pub(crate) message: String,
}

#[derive(serde::Serialize)]
pub(crate) struct SaveChangelistErrorContext {
    pub(crate) errors: Vec<SaveChangelistErrorRow>,
    pub(crate) app: String,
    pub(crate) model: String,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
}

pub(crate) async fn admin_save_changelist_post(
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

    let user = require_perm(&req, db, admin.model_meta(), "change").await?;
    let tenant = resolve_tenant(&user, &tenant_scope, &req)?;

    let editable_fields = admin.list_editable_fields();
    let Form(pairs) = Form::<Vec<(String, String)>>::from_request(&req).await?;

    // Group by pk, allowlisting field names against list_editable_fields() -
    // a key naming a field that isn't configured as editable is silently
    // dropped, not applied and not an error (defense in depth: mirrors
    // list_filter's route-level allowlist check, in case a crafted request
    // names a real-but-not-editable field).
    let mut by_pk: std::collections::HashMap<i64, std::collections::HashMap<String, String>> =
        std::collections::HashMap::new();
    for (key, value) in &pairs {
        let Some(rest) = key.strip_prefix("edit-") else {
            continue;
        };
        let Some((pk_str, field_name)) = rest.split_once('-') else {
            continue;
        };
        let pk = pk_str
            .parse::<i64>()
            .map_err(|_| DjangorsError::BadRequest("invalid pk in edit field".to_string()))?;
        if !editable_fields.contains(&field_name) {
            continue;
        }
        by_pk
            .entry(pk)
            .or_default()
            .insert(field_name.to_string(), value.clone());
    }

    let mut row_errors: Vec<(i64, std::collections::HashMap<String, String>)> = Vec::new();
    for (pk, fields) in &by_pk {
        match admin
            .update_fields_from_form(db, *pk, fields, tenant)
            .await?
        {
            Ok(()) => {
                log_action(
                    db,
                    user.id,
                    admin.model_meta(),
                    *pk,
                    ACTION_CHANGE,
                    "Changed.",
                    None,
                )
                .await?;
            }
            Err(errors) => row_errors.push((*pk, errors)),
        }
    }

    if row_errors.is_empty() {
        return Ok(Response::redirect(&format!("/{}/{}/", app, model)));
    }

    let mut errors = Vec::new();
    for (pk, error_map) in &row_errors {
        for (field, msg) in error_map {
            errors.push(SaveChangelistErrorRow {
                pk: *pk,
                field: field.clone(),
                message: msg.clone(),
            });
        }
    }

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/save_changelist_error.html",
        SaveChangelistErrorContext {
            errors,
            app: app.to_string(),
            model: model.to_string(),
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
        },
    )
}
