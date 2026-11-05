use crate::AdminAction;
use async_trait::async_trait;
use djangors_core::DjangorsError;
use djangors_orm::meta::{Model, ModelMeta};
use std::marker::PhantomData;

/// Struct containing page data for a model's changelist view.
pub struct ChangelistPage {
    /// Field names to display as columns in declaration order.
    pub columns: Vec<&'static str>,
    /// Display-rendered row strings (view escapes).
    pub rows: Vec<Vec<String>>,
    /// Primary key strings corresponding to each row.
    pub pks: Vec<String>,
    /// Total count of matching items across all pages.
    pub total: i64,
    /// 1-based current page number.
    pub page: i64,
    /// Number of items displayed per page.
    pub per_page: i64,
}

/// Configuration for inline child model display on admin forms.
#[derive(Clone)]
pub struct InlineConfig {
    /// The child model's struct name for lookups via `all_registered_models()`.
    pub struct_name: &'static str,
    /// The FK field name on the child model pointing to the parent.
    pub relation_field: &'static str,
    /// Field names to display in the inline table.
    pub fields: &'static [&'static str],
}

/// Configuration options for custom model admin registration.
#[derive(Default, Clone)]
pub struct ModelAdminConfig {
    /// Subset/reorder of real field names to show as changelist columns.
    /// `None` = all fields, declaration order (today's only behavior).
    pub list_display: Option<&'static [&'static str]>,
    /// Real, text-like field names to ILIKE-match against `?q=`.
    /// `None` or empty = no search box rendered, `?q=` ignored if present.
    pub search_fields: Option<&'static [&'static str]>,
    /// Boolean field names only.
    pub list_filter: Option<&'static [&'static str]>,
    /// Date or datetime field name to display date hierarchy navigation bar.
    pub date_hierarchy: Option<&'static str>,
    /// List of field names that are editable inline on the changelist page.
    pub list_editable: Option<&'static [&'static str]>,
    /// Computed display columns functions.
    #[allow(clippy::type_complexity)]
    pub computed_columns: Option<
        &'static [(
            &'static str,
            fn(&[(&'static str, djangors_orm::expr::Value)]) -> String,
        )],
    >,
    /// Bulk actions available on the changelist page.
    pub actions: Option<&'static [AdminAction]>,
    /// Groupings of fields into fieldsets on the change form.
    pub fieldsets: Option<&'static [(&'static str, &'static [&'static str])]>,
    /// List of field names rendered as read-only on the change form.
    pub readonly_fields: Option<&'static [&'static str]>,
    /// Field names excluded from admin forms, changelists, search, filters, and writes.
    pub exclude: Option<&'static [&'static str]>,
    /// List of foreign key field names rendered as raw ID inputs.
    pub raw_id_fields: Option<&'static [&'static str]>,
    /// Base filter expression automatically applied to all admin queries.
    pub base_filter: Option<djangors_orm::UnresolvedExpr>,
    /// Inline child model configurations rendered below the add/change form.
    pub inlines: Option<&'static [InlineConfig]>,
}

/// A resolved, request-scoped tenant restriction: the tenant FK field name on the
/// model plus the tenant id every row must match. `None` means "no restriction"
/// (the admin site is not tenant-scoped, or the acting user is a superuser).
///
/// This is passed *per call* rather than stored on the `ModelAdmin`. A
/// `ModelAdmin` is constructed once at registration and shared across every
/// concurrent request via `Arc`, so storing the active tenant on the admin would
/// let one request's tenant leak into another's query while the first is awaiting
/// the database.
pub type TenantFilter = Option<(&'static str, i64)>;

/// Pluggable administration interface trait for managing ORM models.
#[async_trait]
pub trait ModelAdmin: Send + Sync {
    /// Returns the metadata for the administered model.
    fn model_meta(&self) -> &'static ModelMeta;
    /// Returns the list of model field names in declaration order.
    fn field_names(&self) -> Vec<&'static str>;
    /// Queries and returns a paginated [`ChangelistPage`] for this model.
    #[allow(clippy::too_many_arguments)]
    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>, // raw ?o= value, e.g. "name" or "-name"
        page: i64,           // already-validated >= 1
        per_page: i64,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
    ) -> Result<ChangelistPage, DjangorsError>;

    /// Queries and returns headers and rows for CSV export.
    #[allow(clippy::too_many_arguments)]
    async fn export_csv_rows(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
    ) -> Result<(Vec<&'static str>, Vec<Vec<String>>), DjangorsError>;

    /// Returns search field names for this model admin.
    fn search_fields(&self) -> Vec<&'static str>;
    /// Returns list filter field names for this model admin.
    fn list_filter_fields(&self) -> Vec<&'static str>;
    /// Returns the date hierarchy field name if configured.
    fn date_hierarchy_field(&self) -> Option<&'static str>;
    /// Returns list editable field names for this model admin.
    fn list_editable_fields(&self) -> Vec<&'static str>;
    /// Returns configured bulk admin actions for this model admin.
    fn actions(&self) -> Vec<AdminAction> {
        Vec::new()
    }
    /// Returns configured change form fieldsets.
    fn fieldsets(&self) -> Option<&'static [(&'static str, &'static [&'static str])]> {
        None
    }
    /// Returns read-only field names for the change form.
    fn readonly_fields(&self) -> &[&'static str] {
        &[]
    }
    /// Returns fields that must not be exposed or changed through the admin.
    fn excluded_fields(&self) -> &[&'static str] {
        &[]
    }
    /// Returns raw ID field names for foreign keys.
    fn raw_id_fields(&self) -> &[&'static str] {
        &[]
    }

    /// Returns configured inline child model configurations.
    fn inlines_config(&self) -> &[InlineConfig] {
        &[]
    }

    /// Fetches a single object by primary key as name/value tuples.
    ///
    /// Returns `None` when the row does not exist *or* belongs to another
    /// tenant, so a cross-tenant primary key is indistinguishable from a
    /// missing one.
    async fn get_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, DjangorsError>;

    /// Updates an existing object from form parameters.
    async fn update_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError>;

    /// Updates specific fields of an existing object from form parameters.
    async fn update_fields_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError>;

    /// Creates a new model instance from form parameters.
    ///
    /// When `tenant` is set, the new row's tenant field is forced to that
    /// tenant, so a submitted form can never plant a row in another tenant.
    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<i64, std::collections::HashMap<String, String>>, DjangorsError>;

    /// Deletes a single object by primary key. Returns `false` without deleting
    /// anything when the row belongs to another tenant.
    async fn delete_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<bool, DjangorsError>;
}

/// Builds the `field = tenant_id` comparison for a resolved [`TenantFilter`].
pub(crate) fn tenant_compare(
    tenant: TenantFilter,
) -> Option<djangors_orm::expr::UnresolvedCompare> {
    tenant.map(|(field, id)| djangors_orm::expr::UnresolvedCompare {
        field,
        value: djangors_orm::expr::Value::I64(id),
    })
}

/// Blanket impl so any real Model can be registered with zero boilerplate.
pub struct DefaultModelAdmin<M: Model> {
    pub(crate) config: ModelAdminConfig,
    pub(crate) _marker: PhantomData<M>,
}

impl<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static> DefaultModelAdmin<M> {
    pub(crate) fn effective_columns(&self) -> Vec<&'static str> {
        let columns = self
            .config
            .list_display
            .map(|cols| cols.to_vec())
            .unwrap_or_else(M::field_names);
        columns
            .into_iter()
            .filter(|name| !self.excluded(name))
            .collect()
    }

    fn excluded(&self, name: &str) -> bool {
        self.config
            .exclude
            .is_some_and(|fields| fields.contains(&name))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_filtered_queryset(
        &self,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
    ) -> Result<djangors_orm::queryset::QuerySet<M>, DjangorsError> {
        let mut qs = M::objects();
        if let Some(o) = order {
            let field = o.strip_prefix('-').unwrap_or(o);
            if self.excluded(field) {
                return Err(DjangorsError::BadRequest(format!(
                    "cannot order by excluded field '{field}'"
                )));
            }
            qs = qs.order_by(o).map_err(|e| match e {
                djangors_orm::error::OrmError::FieldNotFound { .. } => {
                    DjangorsError::BadRequest(e.to_string())
                }
                _ => DjangorsError::Internal(e.to_string()),
            })?;
        }
        if let Some(term) = search {
            let fields = self.search_fields();
            if !term.is_empty() && !fields.is_empty() {
                qs = qs
                    .filter_or_icontains(&fields, term)
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            }
        }
        if !filters.is_empty() {
            let compares = filters
                .iter()
                .map(|(field, val)| djangors_orm::expr::UnresolvedCompare {
                    field,
                    value: djangors_orm::expr::Value::Bool(*val),
                })
                .collect();
            qs = qs
                .filter(djangors_orm::expr::UnresolvedExpr::And(compares))
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        if let (Some(field), Some((gte, lt))) = (self.date_hierarchy_field(), date_range) {
            qs = qs
                .filter_datetime_range(field, gte, lt)
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        if let Some(bf) = &self.config.base_filter {
            qs = qs
                .filter(bf.clone())
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        if let Some(compare) = tenant_compare(tenant) {
            qs = qs
                .filter(djangors_orm::expr::UnresolvedExpr::And(vec![compare]))
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        }
        Ok(qs)
    }

    /// Returns a `QuerySet<M>` filtered by primary key `pk` AND, when tenant scoping
    /// is active, the current tenant — so PK-backed reads/writes never cross tenant
    /// boundaries (`get_by_pk`, `update_from_form`, `delete_by_pk`, etc.).
    pub(crate) fn scoped_pk_queryset(
        &self,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<djangors_orm::queryset::QuerySet<M>, DjangorsError> {
        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;
        let mut compares = vec![djangors_orm::expr::UnresolvedCompare {
            field: pk_field.name,
            value: djangors_orm::expr::Value::I64(pk),
        }];
        if let Some(tc) = tenant_compare(tenant) {
            compares.push(tc);
        }
        M::objects()
            .filter(djangors_orm::expr::UnresolvedExpr::And(compares))
            .map_err(|e| DjangorsError::Internal(e.to_string()))
    }

    pub(crate) fn row_values(&self, item: &M, columns: &[&'static str]) -> Vec<String> {
        let field_values = item.field_values();
        columns
            .iter()
            .map(|col| {
                // Check computed columns first
                if let Some(comp_cols) = self.config.computed_columns {
                    if let Some((_, f)) = comp_cols.iter().find(|(n, _)| *n == *col) {
                        return f(&field_values);
                    }
                }
                // Fallback to normal field values
                field_values
                    .iter()
                    .find(|(name, _)| name == col)
                    .map(|(_, v)| v.to_string())
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[async_trait]
impl<M: Model + djangors_orm::error::FromRow + Send + Sync + 'static> ModelAdmin
    for DefaultModelAdmin<M>
{
    fn model_meta(&self) -> &'static ModelMeta {
        M::meta()
    }

    fn field_names(&self) -> Vec<&'static str> {
        M::field_names()
            .into_iter()
            .filter(|name| !self.excluded(name))
            .collect()
    }

    fn search_fields(&self) -> Vec<&'static str> {
        self.config
            .search_fields
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(|name| !self.excluded(name))
            .collect()
    }

    fn list_filter_fields(&self) -> Vec<&'static str> {
        self.config
            .list_filter
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(|name| !self.excluded(name))
            .collect()
    }

    fn date_hierarchy_field(&self) -> Option<&'static str> {
        self.config
            .date_hierarchy
            .filter(|name| !self.excluded(name))
    }

    fn list_editable_fields(&self) -> Vec<&'static str> {
        self.config
            .list_editable
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(|name| !self.excluded(name))
            .collect()
    }

    fn actions(&self) -> Vec<AdminAction> {
        let mut list = self.config.actions.map(|a| a.to_vec()).unwrap_or_default();
        list.push(AdminAction {
            name: "delete_selected",
            label: "Delete selected",
            requires_confirm: true,
            handler: |db: &djangors_db::Database, pks: &[i64]| {
                Box::pin(async move {
                    for &pk in pks {
                        let _ = djangors_orm::queryset::QuerySet::<M>::delete_by_pk(db, pk).await;
                    }
                    Ok(())
                })
            },
        });
        list
    }

    fn fieldsets(&self) -> Option<&'static [(&'static str, &'static [&'static str])]> {
        self.config.fieldsets
    }

    fn readonly_fields(&self) -> &[&'static str] {
        self.config.readonly_fields.unwrap_or(&[])
    }

    fn excluded_fields(&self) -> &[&'static str] {
        self.config.exclude.unwrap_or(&[])
    }

    fn raw_id_fields(&self) -> &[&'static str] {
        self.config.raw_id_fields.unwrap_or(&[])
    }

    fn inlines_config(&self) -> &[InlineConfig] {
        self.config.inlines.unwrap_or(&[])
    }

    #[allow(clippy::too_many_arguments)]
    async fn changelist(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        page: i64,
        per_page: i64,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
    ) -> Result<ChangelistPage, DjangorsError> {
        let mut qs = self.build_filtered_queryset(order, search, filters, date_range, tenant)?;

        let total = qs
            .clone()
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let offset = (page - 1) * per_page;
        qs = qs.limit(per_page).offset(offset);
        let items = qs
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let columns = self.effective_columns();

        let meta = M::meta();
        let pk_field_name = meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .map(|f| f.name)
            .unwrap_or("id");

        let mut rows = Vec::new();
        let mut pks = Vec::new();

        for item in &items {
            let row_vals = self.row_values(item, &columns);
            rows.push(row_vals);

            let field_values = item.field_values();
            let pk_val = field_values
                .iter()
                .find(|(name, _)| name == &pk_field_name)
                .map(|(_, v)| v.to_string())
                .unwrap_or_default();
            pks.push(pk_val);
        }

        Ok(ChangelistPage {
            columns,
            rows,
            pks,
            total,
            page,
            per_page,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn export_csv_rows(
        &self,
        db: &djangors_db::Database,
        order: Option<&str>,
        search: Option<&str>,
        filters: &[(&'static str, bool)],
        date_range: Option<(chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>,
        tenant: TenantFilter,
    ) -> Result<(Vec<&'static str>, Vec<Vec<String>>), DjangorsError> {
        let qs = self.build_filtered_queryset(order, search, filters, date_range, tenant)?;
        let items = qs
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let columns = self.effective_columns();
        let rows = items
            .iter()
            .map(|item| self.row_values(item, &columns))
            .collect();
        Ok((columns, rows))
    }

    async fn get_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<Option<Vec<(&'static str, djangors_orm::expr::Value)>>, DjangorsError> {
        let row_opt = self.scoped_pk_queryset(pk, tenant)?.get(db).await;

        match row_opt {
            Ok(row) => Ok(Some(row.field_values())),
            Err(djangors_orm::error::OrmError::NotFound { .. }) => Ok(None),
            Err(e) => Err(DjangorsError::Internal(e.to_string())),
        }
    }

    async fn update_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError> {
        let meta = M::meta();
        let mut errors = std::collections::HashMap::new();
        let mut sets = Vec::new();

        for field in meta.fields {
            if field.auto || self.excluded(field.name) {
                continue;
            }
            let raw_opt = form.get(field.name).map(|s| s.as_str());
            match parse_field_value(field, raw_opt) {
                Ok(val) => {
                    sets.push((field.name, djangors_orm::expr::SetExpr::Literal(val)));
                }
                Err(err_msg) => {
                    errors.insert(field.name.to_string(), err_msg);
                }
            }
        }

        for rel in meta.relations {
            if self.excluded(rel.field_name) {
                continue;
            }
            let raw_opt = form.get(rel.field_name).map(|s| s.as_str());
            match parse_relation_value(rel, raw_opt) {
                Ok(val) => {
                    sets.push((rel.field_name, djangors_orm::expr::SetExpr::Literal(val)));
                }
                Err(err_msg) => {
                    errors.insert(rel.field_name.to_string(), err_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Ok(Err(errors));
        }
        if sets.is_empty() {
            return Ok(Ok(()));
        }

        let qs = self.scoped_pk_queryset(pk, tenant)?;

        qs.update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(()))
    }

    async fn update_fields_from_form(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<(), std::collections::HashMap<String, String>>, DjangorsError> {
        let meta = M::meta();
        let mut errors = std::collections::HashMap::new();
        let mut sets = Vec::new();

        for field in meta.fields {
            if field.auto || self.excluded(field.name) {
                continue;
            }
            let Some(raw) = form.get(field.name) else {
                continue; // not part of this edit - leave the column untouched
            };
            match parse_field_value(field, Some(raw.as_str())) {
                Ok(val) => sets.push((field.name, djangors_orm::expr::SetExpr::Literal(val))),
                Err(err_msg) => {
                    errors.insert(field.name.to_string(), err_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Ok(Err(errors));
        }
        if sets.is_empty() {
            return Ok(Ok(())); // nothing in `form` matched a real field - a no-op, not an error
        }

        let qs = self.scoped_pk_queryset(pk, tenant)?;
        qs.update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(()))
    }

    async fn create_from_form(
        &self,
        db: &djangors_db::Database,
        form: &std::collections::HashMap<String, String>,
        tenant: TenantFilter,
    ) -> Result<Result<i64, std::collections::HashMap<String, String>>, DjangorsError> {
        let meta = M::meta();
        let mut errors = std::collections::HashMap::new();
        let mut values = Vec::new();

        for field in meta.fields {
            if field.auto || field.primary_key || self.excluded(field.name) {
                continue;
            }
            let raw_opt = form.get(field.name).map(|s| s.as_str());
            match parse_field_value(field, raw_opt) {
                Ok(val) => {
                    values.push((field.name, val));
                }
                Err(err_msg) => {
                    errors.insert(field.name.to_string(), err_msg);
                }
            }
        }

        for rel in meta.relations {
            if self.excluded(rel.field_name) {
                continue;
            }
            let raw_opt = form.get(rel.field_name).map(|s| s.as_str());
            match parse_relation_value(rel, raw_opt) {
                Ok(val) => {
                    values.push((rel.field_name, val));
                }
                Err(err_msg) => {
                    errors.insert(rel.field_name.to_string(), err_msg);
                }
            }
        }

        if !errors.is_empty() {
            return Ok(Err(errors));
        }

        // When tenant scoping is active, force the new row's tenant field to the
        // current tenant so an object created from a scoped add-form always belongs
        // to the right tenant — the form cannot set another tenant's id.
        if let Some((field, id)) = tenant {
            values.push((field, djangors_orm::expr::Value::I64(id)));
        }

        let pk = djangors_orm::queryset::QuerySet::<M>::insert_raw(db, values)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Ok(pk))
    }

    async fn delete_by_pk(
        &self,
        db: &djangors_db::Database,
        pk: i64,
        tenant: TenantFilter,
    ) -> Result<bool, DjangorsError> {
        // If tenant scoping is active, only delete when the row also belongs to the
        // current tenant — otherwise a cross-tenant PK could be deleted.
        if tenant.is_some() {
            let scoped = self
                .scoped_pk_queryset(pk, tenant)?
                .first(db)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            if scoped.is_none() {
                return Ok(false);
            }
        }
        let rows = djangors_orm::queryset::QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(rows > 0)
    }
}

pub(crate) fn parse_field_value(
    field: &djangors_orm::meta::FieldMeta,
    raw: Option<&str>,
) -> Result<djangors_orm::expr::Value, String> {
    use djangors_orm::expr::Value;
    use djangors_orm::meta::FieldKind;

    if field.kind == FieldKind::Boolean {
        match raw {
            Some(val) if !val.is_empty() => return Ok(Value::Bool(true)),
            _ => {
                if field.nullable {
                    return Ok(Value::Null);
                } else {
                    return Ok(Value::Bool(false));
                }
            }
        }
    }

    let raw = raw.unwrap_or("");
    if raw.is_empty() {
        if field.nullable {
            return Ok(Value::Null);
        } else {
            return Err(format!("Field '{}' is required.", field.name));
        }
    }

    match field.kind {
        FieldKind::Char
        | FieldKind::Text
        | FieldKind::Email
        | FieldKind::Url
        | FieldKind::Slug
        | FieldKind::Ip
        | FieldKind::Binary
        | FieldKind::Json => Ok(Value::Text(raw.to_string())),
        FieldKind::FileField => Ok(Value::Text(raw.to_string())),
        FieldKind::Integer | FieldKind::BigInt => raw
            .parse::<i64>()
            .map(Value::I64)
            .map_err(|_| format!("Field '{}' must be a valid integer.", field.name)),
        FieldKind::Float => raw
            .parse::<f64>()
            .map(Value::F64)
            .map_err(|_| format!("Field '{}' must be a valid float.", field.name)),
        FieldKind::Boolean => unreachable!(),
        FieldKind::DateTime => {
            let naive =
                chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S").map_err(|_| {
                    format!(
                        "Field '{}' must be in YYYY-MM-DD HH:MM:SS format.",
                        field.name
                    )
                })?;
            let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc);
            Ok(Value::DateTime(dt))
        }
        FieldKind::Decimal { .. }
        | FieldKind::Date
        | FieldKind::Time
        | FieldKind::Duration
        | FieldKind::Uuid => {
            unreachable!(
                "Unsupported FieldKind {:?} for field '{}' in djangors-admin.",
                field.kind, field.name
            );
        }
    }
}

pub(crate) fn parse_relation_value(
    relation: &djangors_orm::meta::RelationMeta,
    raw: Option<&str>,
) -> Result<djangors_orm::expr::Value, String> {
    use djangors_orm::expr::Value;
    let raw = raw.unwrap_or("");
    if raw.is_empty() {
        return Ok(Value::Null);
    }
    raw.parse::<i64>().map(Value::I64).map_err(|_| {
        format!(
            "Field '{}' must be a valid integer ID.",
            relation.field_name
        )
    })
}
