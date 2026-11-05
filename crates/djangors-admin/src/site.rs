use crate::action_view::admin_bulk_action_post;
use crate::changelist::{admin_changelist, admin_export_csv, admin_save_changelist_post};
use crate::delete_view::{admin_bulk_delete_post, admin_delete_get, admin_delete_post};
use crate::edit_view::{admin_add_get, admin_add_post, admin_change_get, admin_change_post};
use crate::history_view::admin_history;
use crate::index_view::admin_index;
use crate::{DefaultModelAdmin, ModelAdmin, ModelAdminConfig};
use djangors_core::{PathParams, Request, Router};
use djangors_orm::meta::{Model, ModelMeta};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// Custom branding options for the admin interface header and title.
#[derive(Clone)]
pub struct SiteBranding {
    /// Header title displayed at top left of admin pages.
    pub site_header: String,
    /// Browser window/tab title suffix.
    pub site_title: String,
    /// Optional custom logo image URL.
    pub logo_url: Option<String>,
    /// Optional custom CSS accent color hex string.
    pub accent_color: Option<String>,
}

impl Default for SiteBranding {
    fn default() -> Self {
        Self {
            site_header: "Djangors Administration".to_string(),
            site_title: "Djangors site admin".to_string(),
            logo_url: None,
            accent_color: None,
        }
    }
}

/// Tenant-scoping configuration for the admin site.
#[derive(Clone)]
pub struct TenantScope {
    /// The FK field name on every registered model pointing to the Tenant.
    pub tenant_field: &'static str,
    /// Extract the current tenant id from a request. Typically reads
    /// `djangors_contrib_tenancy::CurrentTenant` from request extensions.
    pub extract_tenant_id: fn(&djangors_core::Request) -> Option<i64>,
}

/// Central administration site registry managing registered models and admin views.
pub struct AdminSite {
    // Thread-safe registry containing all registered model administrations.
    pub(crate) registry: Mutex<Vec<Arc<dyn ModelAdmin>>>,
    // Custom branding details (headers, titles, logos, accents) applied to the admin site.
    pub(crate) branding: SiteBranding,
    // Extra user-registered routers to be mounted alongside default admin urls.
    pub(crate) extra_routes: Vec<std::panic::AssertUnwindSafe<Router>>,
    // Optional tenant scoping configuration.
    pub(crate) tenant_scope: Option<TenantScope>,
}

impl Default for AdminSite {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminSite {
    /// Creates a new empty `AdminSite` with default branding.
    pub fn new() -> Self {
        Self {
            registry: Mutex::new(Vec::new()),
            branding: SiteBranding::default(),
            extra_routes: Vec::new(),
            tenant_scope: None,
        }
    }

    /// Sets the admin site header text.
    pub fn with_site_header(mut self, header: impl Into<String>) -> Self {
        self.branding.site_header = header.into();
        self
    }

    /// Sets the admin site title text.
    pub fn with_site_title(mut self, title: impl Into<String>) -> Self {
        self.branding.site_title = title.into();
        self
    }

    /// Sets a custom logo URL for the admin site header.
    pub fn with_logo_url(mut self, url: impl Into<String>) -> Self {
        self.branding.logo_url = Some(url.into());
        self
    }

    /// Sets a custom CSS accent color for the admin site.
    pub fn with_accent_color(mut self, color: impl Into<String>) -> Self {
        self.branding.accent_color = Some(color.into());
        self
    }

    /// Register an extra route directly on the admin site.
    pub fn extra_route(mut self, router: Router) -> Self {
        self.extra_routes.push(std::panic::AssertUnwindSafe(router));
        self
    }

    /// Enable tenant scoping on the admin. When set, every changelist, add,
    /// change, and delete view is restricted to rows belonging to the current
    /// tenant.
    pub fn with_tenant_scoping(
        mut self,
        tenant_field: &'static str,
        extract_tenant_id: fn(&djangors_core::Request) -> Option<i64>,
    ) -> Self {
        self.tenant_scope = Some(TenantScope {
            tenant_field,
            extract_tenant_id,
        });
        self
    }

    /// Register a model with the default (no customization) ModelAdmin.
    pub fn register<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(&self) {
        self.register_with::<M>(ModelAdminConfig::default());
    }

    /// Registers a model `M` with custom [`ModelAdminConfig`].
    pub fn register_with<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static>(
        &self,
        config: ModelAdminConfig,
    ) {
        let meta = M::meta();
        if let Some(excluded) = config.exclude {
            for name in excluded {
                assert!(
                    meta.fields.iter().any(|field| field.name == *name)
                        || meta
                            .relations
                            .iter()
                            .any(|relation| relation.field_name == *name),
                    "exclude field '{}' does not exist on model '{}'",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(list_display) = config.list_display {
            for name in list_display {
                assert!(
                    meta.fields.iter().any(|f| f.name == *name)
                        || meta.relations.iter().any(|r| r.field_name == *name)
                        || config
                            .computed_columns
                            .is_some_and(|cols| cols.iter().any(|(n, _)| *n == *name)),
                    "list_display field '{}' does not exist on model '{}'",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(search_fields) = config.search_fields {
            for name in search_fields {
                let field = meta
                    .fields
                    .iter()
                    .find(|f| f.name == *name)
                    .unwrap_or_else(|| {
                        panic!(
                            "search_fields field '{}' does not exist on model '{}'",
                            name, meta.struct_name
                        )
                    });
                assert!(
                    matches!(
                        field.kind,
                        djangors_orm::meta::FieldKind::Char
                            | djangors_orm::meta::FieldKind::Text
                            | djangors_orm::meta::FieldKind::Email
                            | djangors_orm::meta::FieldKind::Url
                            | djangors_orm::meta::FieldKind::Slug
                            | djangors_orm::meta::FieldKind::Ip
                    ),
                    "search_fields field '{}' on model '{}' is not a text-like field",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(list_filter) = config.list_filter {
            for name in list_filter {
                let field = meta
                    .fields
                    .iter()
                    .find(|f| f.name == *name)
                    .unwrap_or_else(|| {
                        panic!(
                            "list_filter field '{}' does not exist on model '{}'",
                            name, meta.struct_name
                        )
                    });
                let has_choices = !field.choices.is_empty();
                assert!(
                    matches!(field.kind, djangors_orm::meta::FieldKind::Boolean) || has_choices,
                    "list_filter field '{}' on model '{}' is not a Boolean field and has no choices set",
                    name,
                    meta.struct_name
                );
            }
        }
        if let Some(field_name) = config.date_hierarchy {
            let field = meta
                .fields
                .iter()
                .find(|f| f.name == field_name)
                .unwrap_or_else(|| {
                    panic!(
                        "date_hierarchy field '{}' does not exist on model '{}'",
                        field_name, meta.struct_name
                    )
                });
            assert!(
                field.kind == djangors_orm::meta::FieldKind::DateTime,
                "date_hierarchy field '{}' on model '{}' is not a DateTime field",
                field_name,
                meta.struct_name
            );
        }

        if let Some(list_editable) = config.list_editable {
            let effective_columns: Vec<&'static str> = config
                .list_display
                .map(|c| c.to_vec())
                .unwrap_or_else(M::field_names)
                .into_iter()
                .filter(|name| {
                    !config
                        .exclude
                        .is_some_and(|excluded| excluded.contains(name))
                })
                .collect();
            for name in list_editable {
                let field = meta
                    .fields
                    .iter()
                    .find(|f| f.name == *name)
                    .unwrap_or_else(|| {
                        panic!(
                            "list_editable field '{}' does not exist on model '{}'",
                            name, meta.struct_name
                        )
                    });
                assert!(
                    matches!(
                        field.kind,
                        djangors_orm::meta::FieldKind::Char
                            | djangors_orm::meta::FieldKind::Text
                            | djangors_orm::meta::FieldKind::Email
                            | djangors_orm::meta::FieldKind::Url
                            | djangors_orm::meta::FieldKind::Slug
                            | djangors_orm::meta::FieldKind::Integer
                            | djangors_orm::meta::FieldKind::BigInt
                            | djangors_orm::meta::FieldKind::Float
                    ),
                    "list_editable field '{}' on model '{}' is not a supported type \
                     (Boolean and other kinds are not supported in v1 — see design doc)",
                    name,
                    meta.struct_name
                );
                let col_index = effective_columns.iter().position(|c| c == name);
                assert!(
                    col_index.is_some(),
                    "list_editable field '{}' on model '{}' must also be in list_display",
                    name,
                    meta.struct_name
                );
                assert!(
                    col_index != Some(0),
                    "list_editable field '{}' on model '{}' cannot be the first list_display \
                     column (that column is always the row's edit link)",
                    name,
                    meta.struct_name
                );
            }
        }

        if let Some(inlines) = config.inlines {
            for ic in inlines {
                let child_meta = djangors_orm::meta::all_registered_models()
                    .find(|m| m.struct_name == ic.struct_name)
                    .unwrap_or_else(|| {
                        panic!(
                            "inline child model '{}' is not a registered model",
                            ic.struct_name
                        )
                    });
                let rel_exists = child_meta
                    .relations
                    .iter()
                    .any(|r| r.field_name == ic.relation_field);
                assert!(
                    rel_exists,
                    "inline relation_field '{}' does not exist on child model '{}'",
                    ic.relation_field, ic.struct_name
                );
                for fname in ic.fields {
                    let field_exists = child_meta.fields.iter().any(|f| f.name == *fname)
                        || child_meta.relations.iter().any(|r| r.field_name == *fname);
                    assert!(
                        field_exists,
                        "inline field '{}' does not exist on child model '{}'",
                        fname, ic.struct_name
                    );
                }
            }
        }

        let mut reg = self.registry.lock().unwrap();
        reg.push(Arc::new(DefaultModelAdmin::<M> {
            config,
            _marker: PhantomData,
        }));
    }

    /// Build a Router with the index, changelist, and add/change routes.
    ///
    /// Every admin registered here is captured directly in the handler
    /// closures, not via `Router::with_state` — `Router::mount` (the real,
    /// documented way a caller composes `site.urls()` into a larger app
    /// router) only copies routes, never a sub-router's own state, so any
    /// state attached here would silently never reach the handlers once
    /// mounted. Capturing it in the closures sidesteps that entirely: it's
    /// baked into each handler itself, independent of whatever router it
    /// ends up mounted under. `Auth::<User>::from_request` inside the
    /// handlers still relies on `Request::state::<Database>()`, which *is*
    /// expected to come from the caller's own top-level `.with_state(db)`
    /// call (the same state that already correctly reaches every route,
    /// mounted or not, since `Router::dispatch` always attaches whichever
    /// router's `dispatch` is running its own `self.state` — see
    /// `Router::dispatch`).
    pub fn urls(&self) -> Router {
        let reg = self.registry.lock().unwrap();
        let admins: Vec<Arc<dyn ModelAdmin>> = reg.iter().cloned().collect();
        let snapshot: Vec<&'static ModelMeta> =
            admins.iter().map(|item| item.model_meta()).collect();
        let branding = self.branding.clone();

        let index_admins = snapshot.clone();
        let changelist_admins = admins.clone();
        let add_get_admins = admins.clone();
        let add_post_admins = admins.clone();
        let change_get_admins = admins.clone();
        let change_post_admins = admins.clone();
        let delete_get_admins = admins.clone();
        let delete_post_admins = admins.clone();
        let bulk_delete_admins = admins.clone();
        let bulk_action_admins = admins.clone();
        let save_changelist_admins = admins.clone();
        let export_csv_admins = admins.clone();

        let index_branding = branding.clone();
        let changelist_branding = branding.clone();
        let add_get_branding = branding.clone();
        let add_post_branding = branding.clone();
        let change_get_branding = branding.clone();
        let change_post_branding = branding.clone();
        let delete_get_branding = branding.clone();
        let bulk_delete_branding = branding.clone();
        let save_changelist_branding = branding.clone();
        let bulk_action_branding = branding.clone();
        let history_branding = branding.clone();

        let changelist_tenant_scope = self.tenant_scope.clone();
        let export_csv_tenant_scope = self.tenant_scope.clone();
        let add_get_tenant_scope = self.tenant_scope.clone();
        let add_post_tenant_scope = self.tenant_scope.clone();
        let change_get_tenant_scope = self.tenant_scope.clone();
        let change_post_tenant_scope = self.tenant_scope.clone();
        let delete_get_tenant_scope = self.tenant_scope.clone();
        let delete_post_tenant_scope = self.tenant_scope.clone();
        let history_tenant_scope = self.tenant_scope.clone();
        let bulk_delete_tenant_scope = self.tenant_scope.clone();
        let bulk_action_tenant_scope = self.tenant_scope.clone();
        let save_changelist_tenant_scope = self.tenant_scope.clone();

        let mut router = Router::new()
            .get("/", move |req: Request, params: PathParams| {
                admin_index(req, params, index_admins.clone(), index_branding.clone())
            })
            .get(
                "/{app:slug}/{model:slug}/",
                move |req: Request, params: PathParams| {
                    admin_changelist(
                        req,
                        params,
                        changelist_admins.clone(),
                        changelist_branding.clone(),
                        changelist_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/export-csv/",
                move |req: Request, params: PathParams| {
                    admin_export_csv(
                        req,
                        params,
                        export_csv_admins.clone(),
                        export_csv_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/add/",
                move |req: Request, params: PathParams| {
                    admin_add_get(
                        req,
                        params,
                        add_get_admins.clone(),
                        add_get_branding.clone(),
                        add_get_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/add/",
                move |req: Request, params: PathParams| {
                    admin_add_post(
                        req,
                        params,
                        add_post_admins.clone(),
                        add_post_branding.clone(),
                        add_post_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/change/",
                move |req: Request, params: PathParams| {
                    admin_change_get(
                        req,
                        params,
                        change_get_admins.clone(),
                        change_get_branding.clone(),
                        change_get_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/{pk:i64}/change/",
                move |req: Request, params: PathParams| {
                    admin_change_post(
                        req,
                        params,
                        change_post_admins.clone(),
                        change_post_branding.clone(),
                        change_post_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/delete/",
                move |req: Request, params: PathParams| {
                    admin_delete_get(
                        req,
                        params,
                        delete_get_admins.clone(),
                        delete_get_branding.clone(),
                        delete_get_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/{pk:i64}/delete/",
                move |req: Request, params: PathParams| {
                    admin_delete_post(
                        req,
                        params,
                        delete_post_admins.clone(),
                        delete_post_tenant_scope.clone(),
                    )
                },
            )
            .get(
                "/{app:slug}/{model:slug}/{pk:i64}/history/",
                move |req: Request, params: PathParams| {
                    admin_history(
                        req,
                        params,
                        admins.clone(),
                        history_branding.clone(),
                        history_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/bulk-delete/",
                move |req: Request, params: PathParams| {
                    admin_bulk_delete_post(
                        req,
                        params,
                        bulk_delete_admins.clone(),
                        bulk_delete_branding.clone(),
                        bulk_delete_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/save-changelist/",
                move |req: Request, params: PathParams| {
                    admin_save_changelist_post(
                        req,
                        params,
                        save_changelist_admins.clone(),
                        save_changelist_branding.clone(),
                        save_changelist_tenant_scope.clone(),
                    )
                },
            )
            .post(
                "/{app:slug}/{model:slug}/bulk-action/",
                move |req: Request, params: PathParams| {
                    admin_bulk_action_post(
                        req,
                        params,
                        bulk_action_admins.clone(),
                        bulk_action_branding.clone(),
                        bulk_action_tenant_scope.clone(),
                    )
                },
            );

        for extra in self.extra_routes.iter() {
            router = router.mount("", extra.0.clone());
        }
        router
    }
}
