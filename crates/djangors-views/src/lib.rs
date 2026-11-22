#![deny(missing_docs)]
//! Server-rendered generic class-based views for Djangors.
//!
//! This crate provides reusable, generic views that mimic Django's class-based views:
//! - [`ListView`]: Displays multiple records of a model.
//! - [`DetailView`]: Displays a single detailed model instance.
//! - [`CreateView`]: Handles rendering and submission of forms to create a new model instance.
//! - [`UpdateView`]: Handles editing and updating an existing model instance.
//! - [`DeleteView`]: Renders a deletion confirmation page and deletes the object upon POST.
//!
//! Each view takes a [`ViewSetConfig`] specifying the engine, template name, and redirect targets.

use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};
use djangors_orm::expr::{SetExpr, UnresolvedCompare, UnresolvedExpr, Value};
use djangors_orm::{FromRow, Model, ModelForm};
use djangors_template::TemplateEngine;
use hyper::{Method, StatusCode};
use std::collections::HashMap;

/// Configuration shared by generic view handlers.
pub struct ViewSetConfig<'a> {
    /// Template engine used to render responses.
    pub engine: &'a TemplateEngine,
    /// Template used by the view.
    pub template_name: &'a str,
    /// Redirect destination after a successful write.
    pub success_url: &'a str,
}

// Converts a model instance's fields and their values into a JSON object.
// Used to construct template context where fields are exposed under their string names.
fn model_context<M: Model>(model: &M) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, value) in model.field_values() {
        map.insert(
            name.to_string(),
            match value {
                Value::I64(v) => v.into(),
                Value::F64(v) => serde_json::json!(v),
                Value::Text(v) => v.into(),
                Value::Bool(v) => v.into(),
                Value::DateTime(v) => v.to_rfc3339().into(),
                Value::Null => serde_json::Value::Null,
                // Only reachable through a projection such as
                // `QuerySet::values`, never from a plain model field, but
                // render it as an array so a template sees a real list.
                Value::List(items) => serde_json::Value::Array(
                    items
                        .into_iter()
                        .map(|item| match item {
                            Value::I64(v) => v.into(),
                            Value::F64(v) => serde_json::json!(v),
                            Value::Text(v) => v.into(),
                            Value::Bool(v) => v.into(),
                            Value::DateTime(v) => v.to_rfc3339().into(),
                            _ => serde_json::Value::Null,
                        })
                        .collect(),
                ),
            },
        );
    }
    serde_json::Value::Object(map)
}

// Formats a `FormErrors` instance into a JSON object suitable for template rendering.
// Field-specific errors are mapped to their field names, and non-field errors are mapped to `__all__`.
fn errors_context(errors: &djangors_orm::djangors_forms::FormErrors) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (name, error) in &errors.fields {
        map.insert(name.clone(), error.0.clone().into());
    }
    map.insert("__all__".into(), errors.non_field.clone().into());
    serde_json::Value::Object(map)
}

// Helper to look up the primary key field name of the model and extract its `i64` value.
// Fails with an internal error if the primary key field cannot be found or is not an integer.
fn pk_value<M: Model>(model: &M) -> Result<i64, DjangorsError> {
    let name = M::meta()
        .fields
        .iter()
        .find(|f| f.primary_key)
        .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
        .name;
    model
        .field_values()
        .into_iter()
        .find(|(field, _)| *field == name)
        .and_then(|(_, value)| match value {
            Value::I64(v) => Some(v),
            _ => None,
        })
        .ok_or_else(|| DjangorsError::Internal("Primary key is not an integer".into()))
}

// Queries the database to retrieve a single model instance matching the `pk` parameter from path parameters.
async fn find<M: Model + FromRow>(req: &Request, params: &PathParams) -> Result<M, DjangorsError> {
    let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
    let pk: i64 = params.get_as("pk")?;
    let field = M::meta()
        .fields
        .iter()
        .find(|f| f.primary_key)
        .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
        .name;
    M::objects()
        .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
            field,
            value: Value::I64(pk),
        }]))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })
}

// Renders the specified template with the given context and returns a 200 OK HTML response.
fn render(
    engine: &TemplateEngine,
    name: &str,
    context: serde_json::Value,
) -> Result<Response, DjangorsError> {
    engine
        .render(name, context)
        .map(|body| Response::html(StatusCode::OK, body))
        .map_err(|e| DjangorsError::Internal(e.to_string()))
}

/// Generic list view rendering all rows.
pub struct ListView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow> ListView<M> {
    /// Handle a list request.
    pub async fn list(
        req: Request,
        _params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let rows = M::objects()
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        let context =
            serde_json::json!({"object_list": rows.iter().map(model_context).collect::<Vec<_>>()});
        render(config.engine, config.template_name, context)
    }
}

/// Generic detail view rendering one row.
pub struct DetailView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow> DetailView<M> {
    /// Handle a detail request.
    pub async fn detail(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        render(
            config.engine,
            config.template_name,
            serde_json::json!({"object": model_context(&find::<M>(&req, &params).await?)}),
        )
    }
}

/// Generic create view.
pub struct CreateView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow + ModelForm> CreateView<M> {
    /// Handle GET and POST create requests.
    pub async fn create(
        req: Request,
        _params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        if req.method() == Method::GET {
            return render(
                config.engine,
                config.template_name,
                serde_json::json!({"fields": M::field_names(), "errors": {}}),
            );
        }
        let Form(data) = Form::<HashMap<String, String>>::from_request(&req).await?;
        match M::validate_form(&data) {
            Ok(cleaned) => {
                let db = req
                    .state::<djangors_orm::djangors_db::Database>()
                    .ok_or_else(|| {
                        DjangorsError::Internal("Database connection not found".into())
                    })?;
                djangors_orm::QuerySet::<M>::insert_raw(
                    db,
                    M::from_cleaned_form(cleaned).field_values(),
                )
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                Ok(Response::redirect(config.success_url))
            }
            Err(errors) => render(
                config.engine,
                config.template_name,
                serde_json::json!({"fields": M::field_names(), "form": data, "errors": errors_context(&errors)}),
            ),
        }
    }
}

/// Generic update view.
pub struct UpdateView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow + ModelForm> UpdateView<M> {
    /// Handle GET and POST update requests.
    pub async fn update(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let mut object = find::<M>(&req, &params).await?;
        if req.method() == Method::GET {
            return render(
                config.engine,
                config.template_name,
                serde_json::json!({"object": model_context(&object), "fields": M::field_names(), "errors": {}}),
            );
        }
        let Form(data) = Form::<HashMap<String, String>>::from_request(&req).await?;
        match M::validate_form(&data) {
            Ok(cleaned) => {
                let db = req
                    .state::<djangors_orm::djangors_db::Database>()
                    .ok_or_else(|| {
                        DjangorsError::Internal("Database connection not found".into())
                    })?;
                object.apply_cleaned_form(cleaned);
                let pk = pk_value(&object)?;
                let pk_field = M::meta()
                    .fields
                    .iter()
                    .find(|f| f.primary_key)
                    .ok_or_else(|| DjangorsError::Internal("Primary key field not found".into()))?
                    .name;
                let sets = object
                    .field_values()
                    .into_iter()
                    .filter(|(name, _)| *name != pk_field)
                    .map(|(name, value)| (name, SetExpr::Literal(value)))
                    .collect();
                M::objects()
                    .filter(UnresolvedExpr::And(vec![UnresolvedCompare {
                        field: pk_field,
                        value: Value::I64(pk),
                    }]))
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?
                    .update(db, sets)
                    .await
                    .map_err(|e| DjangorsError::Internal(e.to_string()))?;
                Ok(Response::redirect(config.success_url))
            }
            Err(errors) => render(
                config.engine,
                config.template_name,
                serde_json::json!({"object": model_context(&object), "form": data, "errors": errors_context(&errors)}),
            ),
        }
    }
}

/// Generic delete view.
pub struct DeleteView<M>(std::marker::PhantomData<M>);
impl<M: Model + FromRow> DeleteView<M> {
    /// Handle GET confirmation and POST deletion requests.
    pub async fn delete(
        req: Request,
        params: PathParams,
        config: &ViewSetConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let object = find::<M>(&req, &params).await?;
        if req.method() == Method::GET {
            return render(
                config.engine,
                config.template_name,
                serde_json::json!({"object": model_context(&object)}),
            );
        }
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let pk = pk_value(&object)?;
        djangors_orm::QuerySet::<M>::delete_by_pk(db, pk)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(Response::redirect(config.success_url))
    }
}

/// Generic view that renders a template with static/extra context.
///
/// Analogous to Django's `TemplateView`.
pub struct TemplateView<'a> {
    /// Template engine used to render the template.
    pub engine: &'a TemplateEngine,
    /// Template name to render.
    pub template_name: &'a str,
    /// Extra context JSON object merged into the template context.
    pub extra_context: serde_json::Value,
}

impl<'a> TemplateView<'a> {
    /// Create a new `TemplateView` with an empty extra context object.
    pub fn new(engine: &'a TemplateEngine, template_name: &'a str) -> Self {
        Self {
            engine,
            template_name,
            extra_context: serde_json::json!({}),
        }
    }

    /// Attach static/extra context to this `TemplateView` (builder pattern).
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.extra_context = context;
        self
    }

    /// Handle a request by rendering the configured template with `extra_context`.
    pub async fn dispatch(
        &self,
        _req: Request,
        _params: PathParams,
    ) -> Result<Response, DjangorsError> {
        render(self.engine, self.template_name, self.extra_context.clone())
    }
}

/// Generic view that redirects to a target URL.
///
/// Analogous to Django's `RedirectView`, supporting permanent (`301 Moved Permanently`)
/// or temporary (`302 Found`) redirects and optional query string forwarding.
pub struct RedirectView<'a> {
    /// Destination URL for the redirect.
    pub url: &'a str,
    /// If `true`, return `301 Moved Permanently`; otherwise return `302 Found`.
    pub permanent: bool,
    /// If `true`, append the incoming request's query string to the target URL.
    pub query_string: bool,
}

impl<'a> RedirectView<'a> {
    /// Create a temporary (`302 Found`) `RedirectView` without query string forwarding.
    pub fn new(url: &'a str) -> Self {
        Self {
            url,
            permanent: false,
            query_string: false,
        }
    }

    /// Set whether the redirect is permanent (`301`) or temporary (`302`).
    pub fn permanent(mut self, permanent: bool) -> Self {
        self.permanent = permanent;
        self
    }

    /// Set whether to forward the incoming request's query string to the destination URL.
    pub fn query_string(mut self, query_string: bool) -> Self {
        self.query_string = query_string;
        self
    }

    /// Handle a request by returning a redirect response.
    pub async fn dispatch(
        &self,
        req: Request,
        _params: PathParams,
    ) -> Result<Response, DjangorsError> {
        let target = if self.query_string {
            match req.raw_query() {
                Some(q) if !q.is_empty() => {
                    let sep = if self.url.contains('?') { '&' } else { '?' };
                    format!("{}{sep}{q}", self.url)
                }
                _ => self.url.to_string(),
            }
        } else {
            self.url.to_string()
        };

        let status = if self.permanent {
            StatusCode::MOVED_PERMANENTLY
        } else {
            StatusCode::FOUND
        };
        Ok(Response::new(status).header("location", &target))
    }
}

/// Generic view for displaying and validating a form without tying it to a model.
///
/// Analogous to Django's `FormView`: `GET` renders `template_name`; `POST`
/// parses URL-encoded form data and validates it via a user-supplied closure,
/// redirecting to `success_url` on success or re-rendering with errors on failure.
pub struct FormView<'a> {
    /// Template engine used to render the form template.
    pub engine: &'a TemplateEngine,
    /// Template rendered for `GET` and invalid `POST` submissions.
    pub template_name: &'a str,
    /// Redirect target after a valid `POST` submission.
    pub success_url: &'a str,
    /// Additional static context merged into the rendered template context.
    pub extra_context: serde_json::Value,
}

impl<'a> FormView<'a> {
    /// Create a new `FormView`.
    pub fn new(engine: &'a TemplateEngine, template_name: &'a str, success_url: &'a str) -> Self {
        Self {
            engine,
            template_name,
            success_url,
            extra_context: serde_json::json!({}),
        }
    }

    /// Attach extra context to this `FormView` (builder pattern).
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.extra_context = context;
        self
    }

    /// Handle `GET` (render form) and `POST` (validate via `validator` closure).
    pub async fn dispatch<V, T>(
        &self,
        req: Request,
        _params: PathParams,
        validator: V,
    ) -> Result<Response, DjangorsError>
    where
        V: FnOnce(&HashMap<String, String>) -> Result<T, HashMap<String, String>>,
    {
        if req.method() == Method::GET {
            let mut ctx = match self.extra_context.clone() {
                serde_json::Value::Object(map) => map,
                _ => serde_json::Map::new(),
            };
            ctx.insert("form".into(), serde_json::json!({}));
            ctx.insert("errors".into(), serde_json::json!({}));
            return render(
                self.engine,
                self.template_name,
                serde_json::Value::Object(ctx),
            );
        }

        let Form(data) = Form::<HashMap<String, String>>::from_request(&req).await?;
        match validator(&data) {
            Ok(_) => Ok(Response::redirect(self.success_url)),
            Err(errors) => {
                let mut ctx = match self.extra_context.clone() {
                    serde_json::Value::Object(map) => map,
                    _ => serde_json::Map::new(),
                };
                ctx.insert("form".into(), serde_json::json!(data));
                ctx.insert("errors".into(), serde_json::json!(errors));
                render(
                    self.engine,
                    self.template_name,
                    serde_json::Value::Object(ctx),
                )
            }
        }
    }
}

/// Configuration shared by date-based generic archive views.
pub struct DateArchiveConfig<'a> {
    /// Template engine used to render responses.
    pub engine: &'a TemplateEngine,
    /// Template rendered by the archive view.
    pub template_name: &'a str,
    /// Name of the date/datetime field on the model to filter and order by.
    pub date_field: &'static str,
    /// Whether an empty result set renders normally (`true`) or returns `404 Not Found` (`false`).
    pub allow_empty: bool,
}

impl<'a> DateArchiveConfig<'a> {
    /// Create a new `DateArchiveConfig` with `allow_empty` defaulting to `true`.
    pub fn new(
        engine: &'a TemplateEngine,
        template_name: &'a str,
        date_field: &'static str,
    ) -> Self {
        Self {
            engine,
            template_name,
            date_field,
            allow_empty: true,
        }
    }

    /// Configure whether empty archives return `200 OK` (`true`) or `404 Not Found` (`false`).
    pub fn allow_empty(mut self, allow_empty: bool) -> Self {
        self.allow_empty = allow_empty;
        self
    }
}

fn utc_start_of_day(date: chrono::NaiveDate) -> chrono::DateTime<chrono::Utc> {
    let ndt = date.and_hms_opt(0, 0, 0).expect("valid midnight time");
    chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc)
}

/// Top-level date archive view listing all objects ordered newest-first by `date_field`.
pub struct ArchiveIndexView<M>(std::marker::PhantomData<M>);

impl<M: Model + FromRow> ArchiveIndexView<M> {
    /// Handle an archive index request.
    pub async fn list(
        req: Request,
        _params: PathParams,
        config: &DateArchiveConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let order_expr = format!("-{}", config.date_field);
        let rows = M::objects()
            .order_by(&order_expr)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if !config.allow_empty && rows.is_empty() {
            return Err(DjangorsError::NotFound);
        }
        render(
            config.engine,
            config.template_name,
            serde_json::json!({
                "object_list": rows.iter().map(model_context).collect::<Vec<_>>(),
                "date_field": config.date_field,
            }),
        )
    }
}

/// Year archive view listing all objects whose `date_field` falls within `year`.
pub struct YearArchiveView<M>(std::marker::PhantomData<M>);

impl<M: Model + FromRow> YearArchiveView<M> {
    /// Handle a year archive request reading the `year` path parameter.
    pub async fn list(
        req: Request,
        params: PathParams,
        config: &DateArchiveConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let year: i32 = params.get_as("year")?;
        let start_date = chrono::NaiveDate::from_ymd_opt(year, 1, 1)
            .ok_or_else(|| DjangorsError::BadRequest(format!("invalid year: {year}")))?;
        let end_date = chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
            .ok_or_else(|| DjangorsError::BadRequest(format!("invalid year: {year}")))?;
        let order_expr = format!("-{}", config.date_field);
        let rows = M::objects()
            .filter_datetime_range(
                config.date_field,
                utc_start_of_day(start_date),
                utc_start_of_day(end_date),
            )
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .order_by(&order_expr)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if !config.allow_empty && rows.is_empty() {
            return Err(DjangorsError::NotFound);
        }
        render(
            config.engine,
            config.template_name,
            serde_json::json!({
                "object_list": rows.iter().map(model_context).collect::<Vec<_>>(),
                "year": year,
                "date_field": config.date_field,
            }),
        )
    }
}

/// Month archive view listing all objects whose `date_field` falls within `(year, month)`.
pub struct MonthArchiveView<M>(std::marker::PhantomData<M>);

impl<M: Model + FromRow> MonthArchiveView<M> {
    /// Handle a month archive request reading `year` and `month` (1..=12) path parameters.
    pub async fn list(
        req: Request,
        params: PathParams,
        config: &DateArchiveConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let year: i32 = params.get_as("year")?;
        let month: u32 = params.get_as("month")?;
        let start_date = chrono::NaiveDate::from_ymd_opt(year, month, 1).ok_or_else(|| {
            DjangorsError::BadRequest(format!("invalid year/month: {year}-{month}"))
        })?;
        let (next_year, next_month) = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };
        let end_date =
            chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1).ok_or_else(|| {
                DjangorsError::BadRequest(format!("invalid year/month: {year}-{month}"))
            })?;
        let order_expr = format!("-{}", config.date_field);
        let rows = M::objects()
            .filter_datetime_range(
                config.date_field,
                utc_start_of_day(start_date),
                utc_start_of_day(end_date),
            )
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .order_by(&order_expr)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if !config.allow_empty && rows.is_empty() {
            return Err(DjangorsError::NotFound);
        }
        render(
            config.engine,
            config.template_name,
            serde_json::json!({
                "object_list": rows.iter().map(model_context).collect::<Vec<_>>(),
                "year": year,
                "month": month,
                "date_field": config.date_field,
            }),
        )
    }
}

/// Day archive view listing all objects whose `date_field` falls on `(year, month, day)`.
pub struct DayArchiveView<M>(std::marker::PhantomData<M>);

impl<M: Model + FromRow> DayArchiveView<M> {
    /// Handle a day archive request reading `year`, `month`, and `day` path parameters.
    pub async fn list(
        req: Request,
        params: PathParams,
        config: &DateArchiveConfig<'_>,
    ) -> Result<Response, DjangorsError> {
        let db = req.require_state::<djangors_orm::djangors_db::Database>()?;
        let year: i32 = params.get_as("year")?;
        let month: u32 = params.get_as("month")?;
        let day: u32 = params.get_as("day")?;
        let start_date = chrono::NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| {
            DjangorsError::BadRequest(format!("invalid date: {year}-{month}-{day}"))
        })?;
        let end_date = start_date
            .succ_opt()
            .ok_or_else(|| DjangorsError::BadRequest(format!("date out of range: {start_date}")))?;
        let order_expr = format!("-{}", config.date_field);
        let rows = M::objects()
            .filter_datetime_range(
                config.date_field,
                utc_start_of_day(start_date),
                utc_start_of_day(end_date),
            )
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .order_by(&order_expr)
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if !config.allow_empty && rows.is_empty() {
            return Err(DjangorsError::NotFound);
        }
        render(
            config.engine,
            config.template_name,
            serde_json::json!({
                "object_list": rows.iter().map(model_context).collect::<Vec<_>>(),
                "year": year,
                "month": month,
                "day": day,
                "date_field": config.date_field,
            }),
        )
    }
}

#[cfg(test)]
mod tests;
