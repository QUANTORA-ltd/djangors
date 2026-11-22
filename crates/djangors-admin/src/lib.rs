#![deny(missing_docs)]
//! Automatic administration interface for Djangors applications.
//!
//! This crate provides a complete database-driven admin interface, mirroring Django's Admin.
//! Key components:
//! - [`AdminSite`]: Central registry. Holds registered [`ModelAdmin`]s, configures branding, and mounts admin routes.
//! - [`ModelAdmin`]: Trait customising lists, forms, validation, and actions for a registered ORM model.
//! - Dynamic Templates: Core pages are rendered using templates (defined in `ADMIN_TEMPLATES` using `LazyLock`
//!   and `djangors-template`) with CSRF protection enabled.

pub(crate) const CHANGELIST_PER_PAGE: i64 = 100;

pub(crate) static ADMIN_TEMPLATES: std::sync::LazyLock<djangors_template::TemplateEngine> =
    std::sync::LazyLock::new(|| {
        djangors_template::TemplateEngine::from_embedded(&[
            (
                "admin/index.html",
                include_str!("../templates/admin/index.html"),
            ),
            (
                "admin/delete_confirm.html",
                include_str!("../templates/admin/delete_confirm.html"),
            ),
            (
                "admin/bulk_action_confirm.html",
                include_str!("../templates/admin/bulk_action_confirm.html"),
            ),
            (
                "admin/bulk_delete_confirm.html",
                include_str!("../templates/admin/bulk_delete_confirm.html"),
            ),
            (
                "admin/save_changelist_error.html",
                include_str!("../templates/admin/save_changelist_error.html"),
            ),
            (
                "admin/render_form.html",
                include_str!("../templates/admin/render_form.html"),
            ),
            (
                "admin/changelist.html",
                include_str!("../templates/admin/changelist.html"),
            ),
            (
                "admin/object_history.html",
                include_str!("../templates/admin/object_history.html"),
            ),
            (
                "admin/base.html",
                include_str!("../templates/admin/base.html"),
            ),
            (
                "admin/admindocs_index.html",
                include_str!("../templates/admin/admindocs_index.html"),
            ),
            (
                "admin/admindocs_models.html",
                include_str!("../templates/admin/admindocs_models.html"),
            ),
            (
                "admin/admindocs_model_detail.html",
                include_str!("../templates/admin/admindocs_model_detail.html"),
            ),
        ])
        .expect("admin templates are compiled into the binary and must always be valid")
    });

mod action_view;
mod admindocs;
mod autocomplete;
mod changelist;
mod delete_view;
mod edit_view;
mod favicon;
mod forms;
mod history_view;
mod index_view;
mod log;
mod model_admin;
mod perms;
mod site;

pub use delete_view::RelatedObjectSummary;
pub use favicon::favicon_routes;
pub use log::{
    AdminAction, AdminActionRow, FieldDiffItem, LogEntry, ACTION_ADDITION, ACTION_CHANGE,
    ACTION_DELETION,
};
pub use model_admin::{
    ChangelistPage, DefaultModelAdmin, InlineConfig, ModelAdmin, ModelAdminConfig,
    SimpleListFilter, TenantFilter,
};
pub use site::{AdminSite, SiteBranding, TenantScope};

// `tests` is a child module of this one, so private items it exercises have to be reachable
// from the crate root. Anything it reaches for across a module boundary is re-exported here.
#[cfg(test)]
pub(crate) use changelist::date_hierarchy_drilldown_values;

#[cfg(test)]
mod tests;
