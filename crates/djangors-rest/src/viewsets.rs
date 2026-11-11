//! Generic ViewSet controllers and route mounting.

use std::marker::PhantomData;
use std::sync::Arc;

use djangors_core::error::DjangorsError;
use djangors_core::pagination::{decode_cursor, encode_cursor, Paginator};
use djangors_core::path_params::PathParams;
use djangors_core::request::Request;
use djangors_core::response::Response;
use djangors_core::Router;
use djangors_orm::expr::{SetExpr, UnresolvedCompare, UnresolvedExpr, Value};
use djangors_orm::meta::{FieldKind, Model};
use djangors_orm::queryset::QuerySet;
use djangors_orm::FromRow;
use hyper::StatusCode;

use crate::*;

/// Configuration options for a ViewSet endpoint (filtering and ordering allowlists).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ViewSetConfig {
    /// Allowlist of field names that can be filtered via `?field=value` query params.
    pub filterable_fields: &'static [&'static str],
    /// Allowlist of field names that can be ordered via `?ordering=field` / `?ordering=-field` query params.
    pub orderable_fields: &'static [&'static str],
    /// Enables opt-in cursor pagination when `?cursor=` is supplied.
    pub cursor_pagination: bool,
    /// Rows per page for this endpoint. `None` uses [`REST_PER_PAGE`].
    pub page_size: Option<i64>,
    /// When set, clients may override the page size with `?page_size=`, capped
    /// at this value. `None` (the default) ignores the query parameter, so page
    /// size stays entirely server-controlled.
    pub max_page_size: Option<i64>,
}

const _: ViewSetConfig = ViewSetConfig::default();

impl ViewSetConfig {
    /// Returns the default configuration as a const expression.
    pub const fn default() -> Self {
        Self::new()
    }

    /// Creates a const-compatible default configuration for static declarations.
    pub const fn new() -> Self {
        Self {
            filterable_fields: &[],
            orderable_fields: &[],
            cursor_pagination: false,
            page_size: None,
            max_page_size: None,
        }
    }

    /// Sets the filterable-field allowlist.
    pub const fn with_filterable_fields(mut self, fields: &'static [&'static str]) -> Self {
        self.filterable_fields = fields;
        self
    }

    /// Sets the orderable-field allowlist.
    pub const fn with_orderable_fields(mut self, fields: &'static [&'static str]) -> Self {
        self.orderable_fields = fields;
        self
    }

    /// Enables or disables cursor pagination.
    pub const fn with_cursor_pagination(mut self, enabled: bool) -> Self {
        self.cursor_pagination = enabled;
        self
    }

    /// Sets the fixed page size.
    pub const fn with_page_size(mut self, size: Option<i64>) -> Self {
        self.page_size = size;
        self
    }

    /// Sets the maximum page size clients may request.
    pub const fn with_max_page_size(mut self, size: Option<i64>) -> Self {
        self.max_page_size = size;
        self
    }
}

impl Default for ViewSetConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewSetConfig {
    /// Resolve the page size for one request.
    ///
    /// Falls back to [`REST_PER_PAGE`], then applies a client-supplied
    /// `?page_size=` only when [`ViewSetConfig::max_page_size`] opts in. Values
    /// that are unparseable or below 1 are ignored rather than rejected, and
    /// anything above the cap is clamped — a bad page size should not fail an
    /// otherwise valid list request.
    pub fn resolve_page_size(&self, req: &Request) -> i64 {
        let default = self.page_size.unwrap_or(REST_PER_PAGE).max(1);
        let Some(max) = self.max_page_size else {
            return default;
        };
        req.query("page_size")
            .and_then(|raw| raw.parse::<i64>().ok())
            .filter(|n| *n >= 1)
            .map(|n| n.min(max))
            .unwrap_or(default)
    }
}

/// Everything a ViewSet needs beyond its model: the field allowlists, the
/// serializer that shapes bodies, and the pagination strategy.
///
/// [`ViewSetConfig`] carries plain data and stays `Clone`/`PartialEq`; the
/// strategies live here because trait objects cannot participate in those
/// derives. Construct with [`ViewSetOptions::new`] and override what you need:
///
/// ```no_run
/// # use djangors_rest::*;
/// # use djangors_orm::meta::Model;
/// # fn demo<M: Model + djangors_orm::FromRow + Send + Sync + 'static>() {
/// let options = ViewSetOptions::<M>::default()
///     .with_serializer(ModelSerializer::<M>::new(
///         FieldSet::all().read_only(&["id"]).write_only(&["password"]),
///     ))
///     .with_pagination(LimitOffsetPagination::default());
/// # }
/// ```
pub struct ViewSetOptions<M: Model + FromRow> {
    /// Filtering and ordering allowlists, plus cursor opt-in.
    pub config: ViewSetConfig,
    /// Shapes response bodies and parses/validates request bodies.
    pub serializer: Arc<dyn Serializer<M>>,
    /// Decides the row window and the list envelope.
    pub pagination: Arc<dyn Pagination>,
    /// Extra query-string filter backends applied to `list`, in order, after
    /// the [`ViewSetConfig`] allowlist has been applied.
    pub filter_backends: Vec<Arc<dyn crate::filters::FilterBackend<M>>>,
    /// Optional per-user/per-IP rate limit applied to every action.
    pub throttle: Option<crate::throttling::Throttle>,
}

impl<M> Clone for ViewSetOptions<M>
where
    M: Model + FromRow,
{
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            serializer: self.serializer.clone(),
            pagination: self.pagination.clone(),
            filter_backends: self.filter_backends.clone(),
            throttle: self.throttle.clone(),
        }
    }
}

impl<M> Default for ViewSetOptions<M>
where
    M: Model + FromRow + Send + Sync + 'static,
{
    /// The historical behaviour: every field both directions, page-number
    /// pagination at [`REST_PER_PAGE`].
    fn default() -> Self {
        Self {
            config: ViewSetConfig::default(),
            serializer: Arc::new(ModelSerializer::<M>::default()),
            pagination: Arc::new(PageNumberPagination::default()),
            filter_backends: Vec::new(),
            throttle: None,
        }
    }
}

impl<M> ViewSetOptions<M>
where
    M: Model + FromRow + Send + Sync + 'static,
{
    /// Start from `config`, with the default serializer and pagination.
    pub fn new(config: ViewSetConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Use a specific serializer.
    pub fn with_serializer<S: Serializer<M>>(mut self, serializer: S) -> Self {
        self.serializer = Arc::new(serializer);
        self
    }

    /// Use a specific pagination strategy.
    pub fn with_pagination<P: Pagination>(mut self, pagination: P) -> Self {
        self.pagination = Arc::new(pagination);
        self
    }

    /// Replace the field allowlists.
    pub fn with_config(mut self, config: ViewSetConfig) -> Self {
        self.config = config;
        self
    }

    /// Append a query-string filter backend.
    ///
    /// Backends run in the order added, after the [`ViewSetConfig`] exact-match
    /// allowlist, and each one narrows the queryset further.
    pub fn with_filter_backend<B: crate::filters::FilterBackend<M>>(mut self, backend: B) -> Self {
        self.filter_backends.push(Arc::new(backend));
        self
    }

    /// Apply a rate limit to every action on this ViewSet.
    pub fn with_throttle(mut self, throttle: crate::throttling::Throttle) -> Self {
        self.throttle = Some(throttle);
        self
    }
}

/// Reads and parses a JSON request body.
async fn json_body(req: &Request) -> Result<serde_json::Value, DjangorsError> {
    let body_bytes = req.body_bytes().await;
    serde_json::from_slice(body_bytes).map_err(|e| {
        DjangorsError::api(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            format!("Failed to parse JSON body: {e}"),
        )
    })
}

fn parse_filter_value<M: Model>(field_name: &str, raw_val: &str) -> Option<Value> {
    let meta = M::meta();
    if let Some(field) = meta.fields.iter().find(|f| f.name == field_name) {
        match field.kind {
            FieldKind::Integer | FieldKind::BigInt => raw_val.parse::<i64>().ok().map(Value::I64),
            FieldKind::Float => raw_val.parse::<f64>().ok().map(Value::F64),
            FieldKind::Boolean => match raw_val {
                "true" | "1" => Some(Value::Bool(true)),
                "false" | "0" => Some(Value::Bool(false)),
                _ => None,
            },
            FieldKind::DateTime => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw_val) {
                    Some(Value::DateTime(dt.with_timezone(&chrono::Utc)))
                } else if let Ok(naive) =
                    chrono::NaiveDateTime::parse_from_str(raw_val, "%Y-%m-%d %H:%M:%S")
                {
                    let dt = chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                        naive,
                        chrono::Utc,
                    );
                    Some(Value::DateTime(dt))
                } else {
                    None
                }
            }
            FieldKind::Char
            | FieldKind::Text
            | FieldKind::Email
            | FieldKind::Url
            | FieldKind::Slug
            | FieldKind::Ip
            | FieldKind::Binary
            | FieldKind::Json
            | FieldKind::Decimal { .. }
            | FieldKind::Date
            | FieldKind::Time
            | FieldKind::Duration
            | FieldKind::Uuid => Some(Value::Text(raw_val.to_string())),
            FieldKind::FileField => Some(Value::Text(raw_val.to_string())),
        }
    } else if meta.relations.iter().any(|r| r.field_name == field_name) {
        raw_val.parse::<i64>().ok().map(Value::I64)
    } else {
        None
    }
}

/// Generic ViewSet controller for model `M`.
///
/// Implements standard REST CRUD handlers:
/// - `list` (GET): Paginated list of records
/// - `retrieve` (GET /{pk}): Single record details
/// - `create` (POST): Create a new record
/// - `update` (PUT / PATCH /{pk}): Update an existing record
/// - `destroy` (DELETE /{pk}): Remove a record
///
/// # No built-in permission check
///
/// `ViewSet`'s associated functions (`list`, `retrieve`, `create`, `update`,
/// `destroy`, and their `_with_options` variants) perform **no authentication
/// or authorization check of their own** — that is intentional, so a caller
/// can compose any [`Permission`] policy at the mounting layer, including
/// [`AllowAny`] for deliberately public endpoints. Registering these functions
/// as bare route handlers (`router.get("/x", ViewSet::<M>::retrieve)`) mounts
/// a completely unauthenticated endpoint. Always mount through
/// [`viewset_routes`], [`viewset_routes_with_config`],
/// [`viewset_routes_with_permission`], or
/// [`viewset_routes_with_config_and_permission`] — each already wraps every
/// handler in the permission check for you. If you must register one of
/// these associated functions as a bare handler yourself, call
/// `your_permission.has_permission(&req).await` and return
/// [`DjangorsError::Unauthorized`] before delegating, the same way those
/// helpers do internally.
pub struct ViewSet<M: Model + FromRow> {
    _marker: PhantomData<M>,
}

/// A model whose queries must always be constrained by caller-defined scope.
///
/// `scope` has no default implementation. Consequently, attempting to use a model
/// without an implementation with [`ScopedViewSet`] is a compile-time error (the
/// compiler reports that the trait bound `SomeModel: Scoped` is not satisfied).
/// The hook is also called by writes to validate request scope; payload field
/// injection, when needed, should be performed by the application's deserializer.
pub trait Scoped: Model + FromRow + Send + Sync + 'static {
    /// Applies mandatory request-specific filtering to a base queryset.
    fn scope(req: &Request, qs: QuerySet<Self>) -> Result<QuerySet<Self>, DjangorsError>;
}

/// CRUD controller whose model must implement [`Scoped`].
///
/// # No built-in permission check
///
/// Like [`ViewSet`], `ScopedViewSet`'s associated functions (`list_with_config`,
/// `retrieve`, `create`, `update`, `destroy`) perform **no authentication
/// check of their own** — only the [`Scoped::scope`] row filter, which
/// constrains *which rows* are visible (typically "this tenant") but says
/// nothing about *who* the caller is or what role they hold. Registering
/// these functions as bare route handlers gives every authenticated member
/// of that scope full read/write access, regardless of role. Always mount
/// through [`scoped_viewset_routes`] or [`scoped_viewset_routes_with_config`]
/// — both wrap every handler in an `IsAuthenticated` check — and layer your
/// own role check inside [`Scoped::scope`] (or in a custom handler) if
/// writes must be restricted further than "any authenticated tenant member."
pub struct ScopedViewSet<M: Scoped> {
    _marker: PhantomData<M>,
}

impl<M: Scoped> ScopedViewSet<M> {
    /// Lists only records returned by the model's scope.
    pub async fn list(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        Self::list_with_config(req, params, &ViewSetConfig::default()).await
    }

    /// Lists scoped records with custom filtering and ordering configuration.
    pub async fn list_with_config(
        req: Request,
        _params: PathParams,
        config: &ViewSetConfig,
    ) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_db::Database>()?;
        let page = req
            .raw_query()
            .and_then(parse_page_param)
            .unwrap_or(1)
            .max(1);
        let per_page = config.resolve_page_size(&req);
        let mut qs = M::scope(&req, QuerySet::new())?;
        for &field in config.filterable_fields {
            if let Some(val_str) = req.query(field) {
                if let Some(value) = parse_filter_value::<M>(field, val_str) {
                    qs = qs
                        .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                            field,
                            value,
                        }]))
                        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                }
            }
        }
        let mut cursor_ordering: Option<(&'static str, bool)> = None;
        if let Some(ordering) = req.query("ordering") {
            for part in ordering.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                let field = part.strip_prefix('-').unwrap_or(part);
                if config.orderable_fields.contains(&field) {
                    if cursor_ordering.is_none() {
                        cursor_ordering = M::meta()
                            .fields
                            .iter()
                            .find(|f| f.name == field)
                            .map(|f| (f.name, part.starts_with('-')));
                    }
                    qs = qs
                        .order_by(part)
                        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                }
            }
        }
        let total = qs
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if config.cursor_pagination {
            let (order_field, descending) = cursor_ordering.ok_or_else(|| {
                DjangorsError::BadRequest(
                    "Cursor pagination requires an allowlisted ordering field".into(),
                )
            })?;
            let pk_field = M::meta()
                .fields
                .iter()
                .find(|f| f.primary_key)
                .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
                .name;
            // A first request has no `?cursor=` yet (there's nothing to bootstrap it from) -
            // that's the start-of-sequence case, not an error: apply ordering only, skip
            // `.after(...)`. Any subsequent request supplies the cursor from the previous
            // response's `next_cursor`.
            if let Some(raw_cursor) = req.query("cursor") {
                let (cursor_pk, raw_value) = decode_cursor(raw_cursor)
                    .map_err(|e| DjangorsError::BadRequest(e.to_string()))?;
                let raw_value = raw_value.ok_or_else(|| {
                    DjangorsError::BadRequest("Cursor is missing its ordering value".into())
                })?;
                let order_value =
                    parse_filter_value::<M>(order_field, &raw_value).ok_or_else(|| {
                        DjangorsError::BadRequest("Cursor ordering value is invalid".into())
                    })?;
                qs = qs
                    .after(order_field, order_value, pk_field, cursor_pk, descending)
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            }
            let order_spec = if descending {
                format!("-{order_field}")
            } else {
                order_field.to_string()
            };
            let pk_spec = if descending {
                format!("-{pk_field}")
            } else {
                pk_field.to_string()
            };
            qs = qs
                .order_by(&order_spec)
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
                .order_by(&pk_spec)
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            let fetched = qs
                .limit(per_page + 1)
                .all(db)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            let has_next = fetched.len() > per_page as usize;
            let items: Vec<M> = fetched.into_iter().take(per_page as usize).collect();
            let next_cursor = if has_next {
                items.last().map(|item| {
                    let values = item.field_values();
                    let pk = values
                        .iter()
                        .find(|(n, _)| *n == pk_field)
                        .and_then(|(_, v)| match v {
                            Value::I64(n) => Some(*n),
                            _ => None,
                        })
                        .unwrap_or(0);
                    let value = values
                        .iter()
                        .find(|(n, _)| *n == order_field)
                        .map(|(_, v)| v.to_string());
                    encode_cursor(pk, value.as_deref())
                })
            } else {
                None
            };
            let mut buf = Vec::with_capacity(items.len().saturating_mul(128).saturating_add(96));
            CursorPagination {
                page_size: per_page,
                max_page_size: None,
            }
            .write_envelope_with_cursor(
                total,
                &mut |out| {
                    out.push(b'[');
                    for (idx, item) in items.iter().enumerate() {
                        if idx > 0 {
                            out.push(b',');
                        }
                        item.write_json(out);
                    }
                    out.push(b']');
                },
                next_cursor.as_deref(),
                &mut buf,
            );
            return Ok(Response::json_bytes(StatusCode::OK, buf));
        }
        let paginator = Paginator::new(total, per_page);
        let items = qs
            .limit(per_page)
            .offset(paginator.offset(page))
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let mut buf = Vec::with_capacity(items.len().saturating_mul(128).saturating_add(96));
        buf.extend_from_slice(b"{\"count\":");
        djangors_orm::json::write_json_i64(&mut buf, total);
        buf.extend_from_slice(b",\"page\":");
        djangors_orm::json::write_json_i64(&mut buf, page);
        buf.extend_from_slice(b",\"total_pages\":");
        djangors_orm::json::write_json_i64(&mut buf, paginator.total_pages());
        buf.extend_from_slice(b",\"results\":[");
        for (idx, item) in items.iter().enumerate() {
            if idx > 0 {
                buf.push(b',');
            }
            item.write_json(&mut buf);
        }
        buf.extend_from_slice(b"]}");
        Ok(Response::json_bytes(StatusCode::OK, buf))
    }

    /// Retrieves a record only if it is in scope.
    pub async fn retrieve(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_db::Database>()?;
        let pk = params
            .get("pk")
            .ok_or_else(|| DjangorsError::BadRequest("Missing primary key parameter".into()))?
            .parse::<i64>()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".into()))?;
        let field = M::meta()
            .fields
            .iter()
            .find(|f| f.primary_key)
            .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
            .name;
        let qs = M::scope(&req, QuerySet::new())?
            .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                field,
                value: Value::I64(pk),
            }]))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        match qs.get(db).await {
            Ok(item) => Ok(Response::json_bytes(
                StatusCode::OK,
                serialize_to_vec(&item),
            )),
            Err(djangors_orm::error::OrmError::NotFound { .. }) => Err(DjangorsError::NotFound),
            Err(e) => Err(DjangorsError::Internal(e.to_string())),
        }
    }

    /// Creates a record after validating the request scope.
    pub async fn create(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_db::Database>()?;
        let _ = M::scope(&req, QuerySet::new())?;
        let json: serde_json::Value = serde_json::from_slice(req.body_bytes().await)
            .map_err(|e| DjangorsError::BadRequest(format!("Failed to parse JSON body: {e}")))?;
        let vals = match deserialize::<M>(&json) {
            Ok(v) => v,
            Err(errors) => {
                return Response::json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &serde_json::json!({"errors": errors}),
                )
            }
        };
        let pk = QuerySet::<M>::insert_raw(db, vals)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let field = M::meta()
            .fields
            .iter()
            .find(|f| f.primary_key)
            .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
            .name;
        let item = M::scope(&req, QuerySet::new())?
            .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                field,
                value: Value::I64(pk),
            }]))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(Response::json_bytes(
            StatusCode::CREATED,
            serialize_to_vec(&item),
        ))
    }

    /// Updates a record only if it is in scope.
    pub async fn update(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_db::Database>()?;
        let pk = params
            .get("pk")
            .ok_or_else(|| DjangorsError::BadRequest("Missing primary key parameter".into()))?
            .parse::<i64>()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".into()))?;
        let json: serde_json::Value = serde_json::from_slice(req.body_bytes().await)
            .map_err(|e| DjangorsError::BadRequest(format!("Failed to parse JSON body: {e}")))?;
        let vals = match deserialize::<M>(&json) {
            Ok(v) => v,
            Err(errors) => {
                return Response::json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    &serde_json::json!({"errors": errors}),
                )
            }
        };
        let field = M::meta()
            .fields
            .iter()
            .find(|f| f.primary_key)
            .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
            .name;
        let cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field,
            value: Value::I64(pk),
        }]);
        let sets = vals
            .into_iter()
            .map(|(col, val)| (col, SetExpr::Literal(val)))
            .collect();
        if M::scope(&req, QuerySet::new())?
            .filter(cmp.clone())
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            == 0
        {
            return Err(DjangorsError::NotFound);
        }
        let item = M::scope(&req, QuerySet::new())?
            .filter(cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(Response::json_bytes(
            StatusCode::OK,
            serialize_to_vec(&item),
        ))
    }

    /// Deletes a record only if it is in scope.
    pub async fn destroy(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_db::Database>()?;
        let pk = params
            .get("pk")
            .ok_or_else(|| DjangorsError::BadRequest("Missing primary key parameter".into()))?
            .parse::<i64>()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".into()))?;
        let field = M::meta()
            .fields
            .iter()
            .find(|f| f.primary_key)
            .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
            .name;
        let scoped = M::scope(&req, QuerySet::new())?
            .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                field,
                value: Value::I64(pk),
            }]))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if scoped.get(db).await.is_err() {
            return Err(DjangorsError::NotFound);
        }
        let n = QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if n == 0 {
            return Err(DjangorsError::NotFound);
        }
        Ok(Response::bytes(
            StatusCode::NO_CONTENT,
            "text/plain",
            Vec::new(),
        ))
    }
}

impl<M> ViewSet<M>
where
    M: Model + FromRow + Send + Sync + 'static,
{
    /// `GET /` — returns paginated list of records using default configuration.
    pub async fn list(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        Self::list_with_config(req, params, &ViewSetConfig::default()).await
    }

    /// `GET /` — returns paginated list of records with custom filtering and ordering allowlists.
    pub async fn list_with_config(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig,
    ) -> Result<Response, DjangorsError> {
        Self::list_with_options(req, params, &ViewSetOptions::new(config.clone())).await
    }

    /// `GET /` — lists records using an explicit serializer and pagination
    /// strategy. The strategy chooses the row window and the response envelope,
    /// so switching to limit/offset paging needs no change here.
    pub async fn list_with_options(
        req: Request,
        _params: PathParams,
        options: &ViewSetOptions<M>,
    ) -> Result<Response, DjangorsError> {
        let config = &options.config;
        // Throttling runs before any work, so a rejected request costs
        // nothing beyond the counter read.
        if let Some(throttle) = &options.throttle {
            throttle.check(&req).await?;
        }

        let db = req.require_state::<djangors_db::Database>()?;

        let per_page = options.pagination.page_size(&req);

        let mut qs = QuerySet::<M>::new();

        // 1. Parse ?field=value query params for allowlisted filterable_fields
        for &field in config.filterable_fields {
            if let Some(val_str) = req.query(field) {
                if let Some(value) = parse_filter_value::<M>(field, val_str) {
                    let cmp = UnresolvedExpr::And(vec![UnresolvedCompare { field, value }]);
                    qs = qs
                        .filter(cmp)
                        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                }
            }
        }

        // 1b. Apply any configured filter backends (lookup suffixes, search,
        // ordering). These run after the exact-match allowlist so a backend can
        // narrow further, never widen.
        qs = crate::filters::apply_backends(&options.filter_backends, &req, qs)?;

        // 2. Parse ?ordering=field / ?ordering=-field query params for allowlisted orderable_fields
        let mut cursor_ordering: Option<(&'static str, bool)> = None;
        if let Some(ordering_param) = req.query("ordering") {
            for part in ordering_param.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                let clean_field = part.strip_prefix('-').unwrap_or(part);
                if config.orderable_fields.contains(&clean_field) {
                    if cursor_ordering.is_none() {
                        cursor_ordering = M::meta()
                            .fields
                            .iter()
                            .find(|f| f.name == clean_field)
                            .map(|f| (f.name, part.starts_with('-')));
                    }
                    qs = qs
                        .order_by(part)
                        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                }
            }
        }

        let total_items = qs
            .count(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        if config.cursor_pagination {
            let (order_field, descending) = cursor_ordering.ok_or_else(|| {
                DjangorsError::BadRequest(
                    "Cursor pagination requires an allowlisted ordering field".into(),
                )
            })?;
            let pk_field = M::meta()
                .fields
                .iter()
                .find(|f| f.primary_key)
                .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
                .name;
            // A first request has no `?cursor=` yet (there's nothing to bootstrap it from) -
            // that's the start-of-sequence case, not an error: apply ordering only, skip
            // `.after(...)`. Any subsequent request supplies the cursor from the previous
            // response's `next_cursor`.
            if let Some(raw_cursor) = req.query("cursor") {
                let (cursor_pk, raw_value) = decode_cursor(raw_cursor)
                    .map_err(|e| DjangorsError::BadRequest(e.to_string()))?;
                let raw_value = raw_value.ok_or_else(|| {
                    DjangorsError::BadRequest("Cursor is missing its ordering value".into())
                })?;
                let order_value =
                    parse_filter_value::<M>(order_field, &raw_value).ok_or_else(|| {
                        DjangorsError::BadRequest("Cursor ordering value is invalid".into())
                    })?;
                qs = qs
                    .after(order_field, order_value, pk_field, cursor_pk, descending)
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            }
            let order_spec = if descending {
                format!("-{}", order_field)
            } else {
                order_field.to_string()
            };
            let pk_spec = if descending {
                format!("-{}", pk_field)
            } else {
                pk_field.to_string()
            };
            qs = qs
                .order_by(&order_spec)
                .map_err(|e| DjangorsError::Internal(e.to_string()))?
                .order_by(&pk_spec)
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            let items = qs
                .limit(per_page + 1)
                .all(db)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            let has_next = items.len() > per_page as usize;
            let items: Vec<M> = items.into_iter().take(per_page as usize).collect();
            let next_cursor = if has_next {
                items.last().map(|item| {
                    let values = item.field_values();
                    let pk = values
                        .iter()
                        .find(|(n, _)| *n == pk_field)
                        .and_then(|(_, v)| match v {
                            Value::I64(n) => Some(*n),
                            _ => None,
                        })
                        .unwrap_or(0);
                    let value = values
                        .iter()
                        .find(|(n, _)| *n == order_field)
                        .map(|(_, v)| v.to_string());
                    encode_cursor(pk, value.as_deref())
                })
            } else {
                None
            };
            let mut buf = Vec::with_capacity(items.len().saturating_mul(128).saturating_add(96));
            CursorPagination {
                page_size: per_page,
                max_page_size: None,
            }
            .write_envelope_with_cursor(
                total_items,
                &mut |out| options.serializer.write_json_many(&items, out),
                next_cursor.as_deref(),
                &mut buf,
            );
            return Ok(Response::json_bytes(StatusCode::OK, buf));
        }

        let slice = options.pagination.slice(&req, total_items);
        let offset = slice.offset;

        let items = qs
            .limit(slice.limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let mut buf = Vec::with_capacity(items.len().saturating_mul(128).saturating_add(96));
        options.pagination.write_envelope(
            &req,
            total_items,
            &mut |out| options.serializer.write_json_many(&items, out),
            &mut buf,
        );

        Ok(Response::json_bytes(StatusCode::OK, buf))
    }

    /// `GET /{pk}` — returns a single record by primary key, or 404.
    pub async fn retrieve(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        Self::retrieve_with_options(req, params, &ViewSetOptions::default()).await
    }

    /// `GET /{pk}` — returns one record, shaped by an explicit serializer.
    pub async fn retrieve_with_options(
        req: Request,
        params: PathParams,
        options: &ViewSetOptions<M>,
    ) -> Result<Response, DjangorsError> {
        // Throttling runs before any work, so a rejected request costs
        // nothing beyond the counter read.
        if let Some(throttle) = &options.throttle {
            throttle.check(&req).await?;
        }

        let db = req.require_state::<djangors_db::Database>()?;

        let pk_str = params.get("pk").ok_or_else(|| {
            DjangorsError::BadRequest("Missing primary key parameter".to_string())
        })?;
        let pk: i64 = pk_str
            .parse()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".to_string()))?;

        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field: pk_field.name,
            value: Value::I64(pk),
        }]);

        let row_opt = QuerySet::<M>::new()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await;

        match row_opt {
            Ok(item) => Ok(Response::json_bytes(
                StatusCode::OK,
                options.serializer.to_json_bytes(&item),
            )),
            Err(djangors_orm::error::OrmError::NotFound { .. }) => Err(DjangorsError::NotFound),
            Err(e) => Err(DjangorsError::Internal(e.to_string())),
        }
    }

    /// `POST /` — creates a new record from JSON body (201 Created or 422 Unprocessable Entity).
    pub async fn create(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        Self::create_with_options(req, params, &ViewSetOptions::default()).await
    }

    /// `POST /` — creates a record using an explicit serializer.
    ///
    /// Validation failures render as a `422` [`DjangorsError::Api`] whose
    /// `details` is the `{field: [messages]}` map, so a client can attach each
    /// message to the input that caused it.
    pub async fn create_with_options(
        req: Request,
        _params: PathParams,
        options: &ViewSetOptions<M>,
    ) -> Result<Response, DjangorsError> {
        // Throttling runs before any work, so a rejected request costs
        // nothing beyond the counter read.
        if let Some(throttle) = &options.throttle {
            throttle.check(&req).await?;
        }

        let db = req.require_state::<djangors_db::Database>()?;

        let json_val = json_body(&req).await?;
        let field_values = options.serializer.parse(&json_val, false)?;

        let pk = QuerySet::<M>::insert_raw(db, field_values)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field: pk_field.name,
            value: Value::I64(pk),
        }]);

        let created_item = QuerySet::<M>::new()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Response::json_bytes(
            StatusCode::CREATED,
            options.serializer.to_json_bytes(&created_item),
        ))
    }

    /// `PUT /{pk}` / `PATCH /{pk}` — updates an existing record (200 OK, 422 Unprocessable Entity, or 404).
    pub async fn update(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        Self::update_with_options(req, params, &ViewSetOptions::default()).await
    }

    /// `PUT /{pk}` / `PATCH /{pk}` — updates a record using an explicit serializer.
    ///
    /// `PATCH` is treated as a partial write, so omitted fields are left alone;
    /// `PUT` requires every writable column.
    pub async fn update_with_options(
        req: Request,
        params: PathParams,
        options: &ViewSetOptions<M>,
    ) -> Result<Response, DjangorsError> {
        // Throttling runs before any work, so a rejected request costs
        // nothing beyond the counter read.
        if let Some(throttle) = &options.throttle {
            throttle.check(&req).await?;
        }

        let db = req.require_state::<djangors_db::Database>()?;

        let pk_str = params.get("pk").ok_or_else(|| {
            DjangorsError::BadRequest("Missing primary key parameter".to_string())
        })?;
        let pk: i64 = pk_str
            .parse()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".to_string()))?;

        let json_val = json_body(&req).await?;
        let partial = req.method() == hyper::http::Method::PATCH;
        let field_values = options.serializer.parse(&json_val, partial)?;

        let meta = M::meta();
        let pk_field =
            meta.fields.iter().find(|f| f.primary_key).ok_or_else(|| {
                DjangorsError::Internal("Primary key field not found".to_string())
            })?;

        let unresolved_cmp = UnresolvedExpr::And(vec![UnresolvedCompare {
            field: pk_field.name,
            value: Value::I64(pk),
        }]);

        let qs = QuerySet::<M>::new()
            .filter(unresolved_cmp.clone())
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        let sets: Vec<(&'static str, SetExpr)> = field_values
            .into_iter()
            .map(|(col, val)| (col, SetExpr::Literal(val)))
            .collect();

        let updated_rows = qs
            .update(db, sets)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        if updated_rows == 0 {
            return Err(DjangorsError::NotFound);
        }

        let updated_item = QuerySet::<M>::new()
            .filter(unresolved_cmp)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        Ok(Response::json_bytes(
            StatusCode::OK,
            options.serializer.to_json_bytes(&updated_item),
        ))
    }

    /// `DELETE /{pk}` — deletes a record by primary key (204 No Content or 404).
    pub async fn destroy(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_db::Database>()?;

        let pk_str = params.get("pk").ok_or_else(|| {
            DjangorsError::BadRequest("Missing primary key parameter".to_string())
        })?;
        let pk: i64 = pk_str
            .parse()
            .map_err(|_| DjangorsError::BadRequest("Invalid primary key".to_string()))?;

        let deleted_count = QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;

        if deleted_count == 0 {
            return Err(DjangorsError::NotFound);
        }

        Ok(Response::bytes(
            StatusCode::NO_CONTENT,
            "text/plain",
            Vec::new(),
        ))
    }
}

fn parse_page_param(query: &str) -> Option<i64> {
    for pair in query.split('&') {
        if let Some((key, val)) = pair.split_once('=') {
            if key == "page" {
                return val.parse::<i64>().ok();
            }
        }
    }
    None
}

/// Mounts standard REST routes for model `M` onto `router` at `base_path`.
///
/// Route layout:
/// - `GET {base_path}` -> list
/// - `POST {base_path}` -> create
/// - `GET {base_path}/{pk:i64}` -> retrieve
/// - `PUT {base_path}/{pk:i64}` -> update
/// - `PATCH {base_path}/{pk:i64}` -> update
/// - `DELETE {base_path}/{pk:i64}` -> destroy
///
pub fn viewset_routes<M>(router: Router, base_path: &str) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
{
    viewset_routes_with_config_and_permission::<M, IsAuthenticated>(
        router,
        base_path,
        ViewSetConfig::default(),
        IsAuthenticated,
    )
}

/// Mounts standard, mandatory-scoped REST routes for model `M`.
pub fn scoped_viewset_routes<M>(router: Router, base_path: &str) -> Router
where
    M: Scoped,
{
    let clean = base_path.trim_end_matches('/');
    let detail = format!("{clean}/{{pk:i64}}");
    let list = if clean.is_empty() { "/" } else { clean };
    let permission = Arc::new(IsAuthenticated);
    router
        .get(list, guarded(permission.clone(), ScopedViewSet::<M>::list))
        .post(
            list,
            guarded(permission.clone(), ScopedViewSet::<M>::create),
        )
        .get(
            &detail,
            guarded(permission.clone(), ScopedViewSet::<M>::retrieve),
        )
        .put(
            &detail,
            guarded(permission.clone(), ScopedViewSet::<M>::update),
        )
        .route(
            &detail,
            hyper::http::Method::PATCH,
            guarded(permission.clone(), ScopedViewSet::<M>::update),
        )
        .delete(&detail, guarded(permission, ScopedViewSet::<M>::destroy))
}

/// Mounts standard REST routes with an explicit permission policy.
///
/// [`viewset_routes`] uses [`IsAuthenticated`] by default. Pass [`AllowAny`] here only for
/// endpoints that are intentionally public.
pub fn viewset_routes_with_permission<M, P>(
    router: Router,
    base_path: &str,
    permission: P,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
    P: Permission,
{
    viewset_routes_with_config_and_permission::<M, P>(
        router,
        base_path,
        ViewSetConfig::default(),
        permission,
    )
}

/// Mounts standard REST routes with custom filtering and ordering configuration.
pub fn viewset_routes_with_config<M>(
    router: Router,
    base_path: &str,
    config: ViewSetConfig,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
{
    viewset_routes_with_config_and_permission::<M, IsAuthenticated>(
        router,
        base_path,
        config,
        IsAuthenticated,
    )
}

/// Mounts mandatory-scoped REST routes with custom filtering and ordering
/// configuration.
///
/// The model must implement [`Scoped`]. Unlike [`scoped_viewset_routes`], this
/// variant sends the [`ViewSetConfig`] to the list handler so filterable and
/// orderable allowlists apply. The create/retrieve/update/destroy actions use
/// the default config (the scope alone provides the row-level restriction).
pub fn scoped_viewset_routes_with_config<M>(
    router: Router,
    base_path: &str,
    config: ViewSetConfig,
) -> Router
where
    M: Scoped,
{
    let clean_base = base_path.trim_end_matches('/');
    let detail_path = format!("{clean_base}/{{pk:i64}}");
    let list_create_path = if clean_base.is_empty() {
        "/"
    } else {
        clean_base
    };
    let permission = Arc::new(IsAuthenticated);
    let list_config = Arc::new(config);

    let list_permission = permission.clone();
    let list_handler = {
        let list_config = list_config.clone();
        move |req: Request, params: PathParams| {
            let perm = list_permission.clone();
            let cfg = list_config.clone();
            async move {
                if !perm.has_permission(&req).await {
                    return Err(DjangorsError::Unauthorized("not authenticated".to_string()));
                }
                ScopedViewSet::<M>::list_with_config(req, params, &cfg).await
            }
        }
    };

    router
        .get(list_create_path, list_handler)
        .post(
            list_create_path,
            guarded(permission.clone(), ScopedViewSet::<M>::create),
        )
        .get(
            &detail_path,
            guarded(permission.clone(), ScopedViewSet::<M>::retrieve),
        )
        .put(
            &detail_path,
            guarded(permission.clone(), ScopedViewSet::<M>::update),
        )
        .route(
            &detail_path,
            hyper::http::Method::PATCH,
            guarded(permission.clone(), ScopedViewSet::<M>::update),
        )
        .delete(
            &detail_path,
            guarded(permission, ScopedViewSet::<M>::destroy),
        )
}

/// Mounts standard REST routes with custom configuration and an explicit permission policy.
pub fn viewset_routes_with_config_and_permission<M, P>(
    router: Router,
    base_path: &str,
    config: ViewSetConfig,
    permission: P,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
    P: Permission,
{
    let clean_base = base_path.trim_end_matches('/');
    let detail_path = format!("{clean_base}/{{pk:i64}}");
    let list_create_path = if clean_base.is_empty() {
        "/"
    } else {
        clean_base
    };
    let permission = Arc::new(permission);
    let config = Arc::new(config);

    let list_permission = permission.clone();
    let list_config = config.clone();
    let list_handler = move |req: Request, params: PathParams| {
        let perm = list_permission.clone();
        let cfg = list_config.clone();
        async move {
            if !perm.has_permission(&req).await {
                return Err(DjangorsError::Unauthorized("not authenticated".to_string()));
            }
            ViewSet::<M>::list_with_config(req, params, &cfg).await
        }
    };

    router
        .get(list_create_path, list_handler)
        .post(
            list_create_path,
            guarded(permission.clone(), ViewSet::<M>::create),
        )
        .get(
            &detail_path,
            guarded(permission.clone(), ViewSet::<M>::retrieve),
        )
        .put(
            &detail_path,
            guarded(permission.clone(), ViewSet::<M>::update),
        )
        .route(
            &detail_path,
            hyper::http::Method::PATCH,
            guarded(permission.clone(), ViewSet::<M>::update),
        )
        .delete(&detail_path, guarded(permission, ViewSet::<M>::destroy))
}

/// Mounts standard REST routes with a full [`ViewSetOptions`] — field
/// allowlists, serializer, and pagination strategy — plus a permission policy.
///
/// This is the most complete mounting function; every other `viewset_routes*`
/// helper is a shorthand that fills in defaults and delegates here.
///
/// Unlike the `_with_config` variants, the options reach *every* handler, so a
/// serializer's read/write field split applies to `create` and `update` as well
/// as to `list` and `retrieve`.
pub fn viewset_routes_with_options<M, P>(
    router: Router,
    base_path: &str,
    options: ViewSetOptions<M>,
    permission: P,
) -> Router
where
    M: Model + FromRow + Send + Sync + 'static,
    P: Permission,
{
    let clean_base = base_path.trim_end_matches('/');
    let detail_path = format!("{clean_base}/{{pk:i64}}");
    let list_create_path = if clean_base.is_empty() {
        "/"
    } else {
        clean_base
    };
    let permission = Arc::new(permission);
    let options = Arc::new(options);

    /// Wraps one options-aware handler with its permission check.
    macro_rules! handler {
        ($method:ident) => {{
            let perm = permission.clone();
            let opts = options.clone();
            move |req: Request, params: PathParams| {
                let perm = perm.clone();
                let opts = opts.clone();
                async move {
                    if !perm.has_permission(&req).await {
                        return Err(DjangorsError::Unauthorized("not authenticated".to_string()));
                    }
                    ViewSet::<M>::$method(req, params, &opts).await
                }
            }
        }};
    }

    router
        .get(list_create_path, handler!(list_with_options))
        .post(list_create_path, handler!(create_with_options))
        .get(&detail_path, handler!(retrieve_with_options))
        .put(&detail_path, handler!(update_with_options))
        .route(
            &detail_path,
            hyper::http::Method::PATCH,
            handler!(update_with_options),
        )
        .delete(&detail_path, guarded(permission, ViewSet::<M>::destroy))
}
