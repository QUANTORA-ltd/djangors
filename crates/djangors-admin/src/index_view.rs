use crate::log::{LogEntry, ACTION_ADDITION, ACTION_DELETION};
use crate::perms::{action_codename, require_staff};
use crate::{SiteBranding, ADMIN_TEMPLATES};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_orm::meta::ModelMeta;

#[derive(serde::Serialize)]
pub(crate) struct IndexModelLink {
    pub(crate) href: minijinja::value::Value,
    pub(crate) label: minijinja::value::Value,
    pub(crate) has_view_permission: bool,
    pub(crate) has_change_permission: bool,
    pub(crate) has_add_permission: bool,
}

#[derive(serde::Serialize)]
pub(crate) struct RecentActionRow {
    pub(crate) action_label: &'static str,
    pub(crate) object_repr: String,
    pub(crate) app_label: String,
    pub(crate) model_name: String,
    pub(crate) action_time: String,
}

#[derive(serde::Serialize)]
pub(crate) struct IndexContext {
    pub(crate) models: Vec<IndexModelLink>,
    pub(crate) recent_actions: Vec<RecentActionRow>,
    pub(crate) site_header: String,
    pub(crate) site_title: String,
    pub(crate) logo_url: Option<String>,
    pub(crate) accent_color: Option<String>,
    pub(crate) impersonation_banner: Option<String>,
}

pub(crate) async fn admin_index(
    req: Request,
    _params: PathParams,
    registry: Vec<&'static ModelMeta>,
    branding: SiteBranding,
) -> Result<Response, DjangorsError> {
    let user = require_staff(&req).await?;

    let db = req.require_state::<djangors_db::Database>()?;

    let mut models = Vec::new();
    for meta in &registry {
        let has_view = if user.is_superuser {
            true
        } else {
            let codename = action_codename(meta, "view");
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
        let has_add = if user.is_superuser {
            true
        } else {
            let codename = action_codename(meta, "add");
            djangors_auth::has_perm(db, user.id, &codename)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
        };
        if !has_view && !has_change && !has_add {
            continue;
        }
        models.push(IndexModelLink {
            href: minijinja::value::Value::from_safe_string(format!(
                "{}/{}/",
                meta.app_label,
                meta.struct_name.to_lowercase()
            )),
            label: minijinja::value::Value::from_safe_string(format!(
                "{}.{}",
                meta.app_label, meta.struct_name
            )),
            has_view_permission: has_view,
            has_change_permission: has_change,
            has_add_permission: has_add,
        });
    }

    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!(
        "SELECT id, user_id, action_time, app_label, model_name, object_id, object_repr, \
         action_flag, change_message, field_diff FROM djangors_admin_log WHERE user_id = {p1} \
         ORDER BY action_time DESC LIMIT 10"
    );
    let params = vec![djangors_db::BindValue::I64(user.id)];
    let db_rows_res = conn.fetch_all(&sql, &params).await;

    let recent_rows = match db_rows_res {
        Ok(rows) => {
            let mut entries = Vec::with_capacity(rows.len());
            let mut err = None;
            for r in &rows {
                let id = match r.try_i64(0) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let user_id = match r.try_i64(1) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let action_time = match r.try_datetime(2) {
                    Ok(Some(v)) => v,
                    Ok(None) => {
                        err = Some(sqlx::Error::Decode("missing action_time".into()));
                        break;
                    }
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let app_label = match r.try_string(3) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let model_name = match r.try_string(4) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let object_id = match r.try_i64(5) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let object_repr = match r.try_string(6) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let action_flag = match r.try_i64(7) {
                    Ok(v) => v.unwrap_or_default() as i32,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let change_message = match r.try_string(8) {
                    Ok(v) => v.unwrap_or_default(),
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                let field_diff = match r.try_string(9) {
                    Ok(v) => v,
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                };
                entries.push(LogEntry {
                    id,
                    user_id,
                    action_time,
                    app_label,
                    model_name,
                    object_id,
                    object_repr,
                    action_flag,
                    change_message,
                    field_diff,
                });
            }
            if let Some(e) = err {
                return Err(DjangorsError::Internal(e.to_string()));
            }
            entries
        }
        Err(e) => {
            let err_str = e.to_string();
            if err_str.contains("djangors_admin_log")
                && (err_str.contains("does not exist") || err_str.contains("no such table"))
            {
                Vec::new()
            } else {
                return Err(DjangorsError::Internal(err_str));
            }
        }
    };

    let recent_actions: Vec<RecentActionRow> = recent_rows
        .into_iter()
        .map(|e| RecentActionRow {
            action_label: match e.action_flag {
                ACTION_ADDITION => "Added",
                ACTION_DELETION => "Deleted",
                _ => "Changed",
            },
            object_repr: e.object_repr,
            app_label: e.app_label,
            model_name: e.model_name,
            action_time: e.action_time.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    let banner = djangors_auth::impersonation_banner_html(&req);
    let impersonation_banner = (!banner.is_empty()).then_some(banner);

    djangors_template::render(
        &ADMIN_TEMPLATES,
        "admin/index.html",
        IndexContext {
            models,
            recent_actions,
            site_header: branding.site_header,
            site_title: branding.site_title,
            logo_url: branding.logo_url,
            accent_color: branding.accent_color,
            impersonation_banner,
        },
    )
}
