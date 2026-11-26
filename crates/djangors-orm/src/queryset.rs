use crate::error::{FromRow, OrmError};
use crate::expr::{ArithOp, CompareOp, Expr, SetExpr, UnresolvedExpr, Value};
use crate::meta::{FieldKind, Model, ModelMeta};
use djangors_db::{BindValue, DbRow, Dialect, NullKind};
use std::marker::PhantomData;

/// Resolves the correct [`NullKind`] for a field on `meta` by name.
fn null_bind_kind_for(field_name: &str, meta: &'static ModelMeta) -> NullKind {
    match meta.fields.iter().find(|f| f.name == field_name) {
        Some(f) => match f.kind {
            FieldKind::Char
            | FieldKind::Text
            | FieldKind::Email
            | FieldKind::Url
            | FieldKind::Slug
            | FieldKind::Ip
            | FieldKind::FileField => NullKind::Text,
            FieldKind::Integer | FieldKind::BigInt => NullKind::I64,
            FieldKind::Float => NullKind::F64,
            FieldKind::Boolean => NullKind::Bool,
            FieldKind::DateTime => NullKind::DateTime,
            _ => NullKind::I64,
        },
        None => NullKind::I64,
    }
}

/// Options for row locking via [`QuerySet::select_for_update`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForUpdate {
    /// If true, return an error immediately if a lock cannot be acquired.
    pub nowait: bool,
    /// If true, skip locked rows.
    pub skip_locked: bool,
    /// Specific tables to lock (Postgres `FOR UPDATE OF`).
    pub of: Vec<String>,
}

/// SQL set operation combining two querysets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOp {
    /// `UNION` (deduplicated).
    Union,
    /// `UNION ALL` (preserves duplicates).
    UnionAll,
    /// `INTERSECT`.
    Intersection,
    /// `EXCEPT`.
    Difference,
}

/// Lazily constructed database query builder for model `T`.
#[derive(Debug)]
pub struct QuerySet<T: Model + FromRow> {
    filters: Vec<Expr>,
    order_by: Vec<(String, bool)>, // (column, descending)
    limit: Option<i64>,
    offset: Option<i64>,
    for_update: Option<ForUpdate>,
    search_query: Option<(Vec<&'static str>, String)>,
    fts5_query: Option<(String, String)>,
    vector_order: Option<(String, crate::vector::VectorDistanceMetric, crate::vector::Vector)>,
    cache_ttl: Option<std::time::Duration>,
    include_deleted: bool,
    distinct: bool,
    distinct_on: Vec<String>,
    defer_fields: Vec<String>,
    only_fields: Vec<String>,
    is_none: bool,
    set_ops: Vec<(SetOp, QuerySet<T>)>,
    _marker: PhantomData<T>,
}

impl<T: Model + FromRow> Clone for QuerySet<T> {
    fn clone(&self) -> Self {
        Self {
            filters: self.filters.clone(),
            order_by: self.order_by.clone(),
            limit: self.limit,
            offset: self.offset,
            for_update: self.for_update.clone(),
            search_query: self.search_query.clone(),
            fts5_query: self.fts5_query.clone(),
            vector_order: self.vector_order.clone(),
            cache_ttl: self.cache_ttl,
            include_deleted: self.include_deleted,
            distinct: self.distinct,
            distinct_on: self.distinct_on.clone(),
            defer_fields: self.defer_fields.clone(),
            only_fields: self.only_fields.clone(),
            is_none: self.is_none,
            set_ops: self.set_ops.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T: Model + FromRow> Default for QuerySet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Model + FromRow> QuerySet<T> {
    /// Creates a new empty `QuerySet` for model `T`.
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            order_by: Vec::new(),
            limit: None,
            offset: None,
            for_update: None,
            search_query: None,
            fts5_query: None,
            vector_order: None,
            cache_ttl: None,
            include_deleted: false,
            distinct: false,
            distinct_on: Vec::new(),
            defer_fields: Vec::new(),
            only_fields: Vec::new(),
            is_none: false,
            set_ops: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Eliminates duplicate rows from the query results (`SELECT DISTINCT`).
    pub fn distinct(mut self) -> Self {
        self.distinct = true;
        self
    }

    /// PostgreSQL `SELECT DISTINCT ON (col1, col2, ...)` — deduplicates by specific fields.
    pub fn distinct_on(mut self, fields: &[&str]) -> Result<Self, OrmError> {
        let meta = T::meta();
        let mut cols = Vec::with_capacity(fields.len());
        for &field in fields {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field) {
                cols.push(format!("\"{}\"", f.column_name));
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field) {
                cols.push(format!("\"{}\"", r.field_name));
            } else {
                return Err(OrmError::FieldNotFound {
                    field: field.to_string(),
                    model: meta.struct_name,
                });
            }
        }
        self.distinct = true;
        self.distinct_on = cols;
        Ok(self)
    }

    /// Defers loading the specified fields from `SELECT` (Django's `QuerySet.defer()`).
    pub fn defer(mut self, fields: &[&str]) -> Result<Self, OrmError> {
        let meta = T::meta();
        for &field in fields {
            if !meta.fields.iter().any(|f| f.name == field)
                && !meta.relations.iter().any(|r| r.field_name == field)
            {
                return Err(OrmError::FieldNotFound {
                    field: field.to_string(),
                    model: meta.struct_name,
                });
            }
            self.defer_fields.push(field.to_string());
        }
        Ok(self)
    }

    /// Loads only the specified fields in `SELECT` (always keeping the primary key).
    pub fn only(mut self, fields: &[&str]) -> Result<Self, OrmError> {
        let meta = T::meta();
        for &field in fields {
            if !meta.fields.iter().any(|f| f.name == field)
                && !meta.relations.iter().any(|r| r.field_name == field)
            {
                return Err(OrmError::FieldNotFound {
                    field: field.to_string(),
                    model: meta.struct_name,
                });
            }
            self.only_fields.push(field.to_string());
        }
        Ok(self)
    }

    /// Marks this queryset as unconditionally empty (Django's `QuerySet.none()`).
    pub fn none(mut self) -> Self {
        self.is_none = true;
        self
    }

    /// Reverses the ordering of this queryset (Django's `QuerySet.reverse()`).
    pub fn reverse(mut self) -> Self {
        if self.order_by.is_empty() {
            let meta = T::meta();
            for &field in meta.ordering {
                let (clean_field, desc) = if let Some(stripped) = field.strip_prefix('-') {
                    (stripped, true)
                } else {
                    (field, false)
                };
                let col = if let Some(f) = meta.fields.iter().find(|f| f.name == clean_field) {
                    format!("\"{}\"", f.column_name)
                } else {
                    format!("\"{}\"", clean_field)
                };
                self.order_by.push((col, !desc));
            }
        } else {
            for (_, desc) in &mut self.order_by {
                *desc = !*desc;
            }
        }
        self
    }

    /// Combines this queryset with `other` using SQL `UNION` (or `UNION ALL` if `all` is `true`).
    pub fn union(mut self, other: QuerySet<T>, all: bool) -> Self {
        self.set_ops
            .push((if all { SetOp::UnionAll } else { SetOp::Union }, other));
        self
    }

    /// Combines this queryset with `other` using SQL `INTERSECT`.
    pub fn intersection(mut self, other: QuerySet<T>) -> Self {
        self.set_ops.push((SetOp::Intersection, other));
        self
    }

    /// Combines this queryset with `other` using SQL `EXCEPT`.
    pub fn difference(mut self, other: QuerySet<T>) -> Self {
        self.set_ops.push((SetOp::Difference, other));
        self
    }

    /// Resolves the database pool to use for this model via a [`djangors_db::MultiDatabase`]
    /// router, or an explicit alias (`QuerySet::using`).
    pub fn using<'a>(
        &self,
        multi: &'a djangors_db::MultiDatabase,
        alias: Option<&str>,
    ) -> Result<&'a djangors_db::Database, OrmError> {
        let db = match alias {
            Some(a) => multi.get(a),
            None => multi.read_db(T::meta().table_name),
        };
        db.map_err(|e| OrmError::InvalidQuery(e.to_string()))
    }

    /// Includes soft-deleted rows in the results of this queryset.
    ///
    /// Has no effect on models without `#[djangors(soft_delete)]`.
    pub fn with_deleted(mut self) -> Self {
        self.include_deleted = true;
        self
    }

    /// Restricts this queryset to soft-deleted rows (`deleted_at IS NOT NULL`).
    ///
    /// On a model without `#[djangors(soft_delete)]` nothing is deleted, so the
    /// queryset matches no rows.
    pub fn only_deleted(mut self) -> Self {
        self.include_deleted = true;
        let filter = match T::soft_delete_column() {
            Some(col) => match T::meta().fields.iter().find(|f| f.column_name == col) {
                Some(f) => Expr::Compare {
                    field: f.name,
                    op: CompareOp::IsNull,
                    value: Value::Bool(false),
                },
                None => Expr::Or(Vec::new()),
            },
            None => Expr::Or(Vec::new()),
        };
        self.filters.push(filter);
        self
    }

    /// Enables result caching for this queryset with an optional TTL.
    ///
    /// Subsequent calls to [`cached_all`](Self::cached_all) reuse the cached
    /// rows until the TTL expires or the model's table is invalidated by a
    /// write (save/update/delete). Requires `T: Clone + Send + Sync + 'static`.
    pub fn cache(mut self, ttl: std::time::Duration) -> Self {
        self.cache_ttl = Some(ttl);
        self
    }

    /// Adds filter expressions generated by `q!` to this queryset.
    pub fn filter(mut self, expr: impl Into<UnresolvedExpr>) -> Result<Self, OrmError> {
        let resolved = Self::resolve_expr(expr.into())?;
        self.filters.push(resolved);
        Ok(self)
    }

    /// The `SELECT` this queryset would execute, for debugging and tests —
    /// Django's `str(queryset.query)`.
    pub fn debug_sql(&self) -> String {
        self.compile_select_with_order("*", true, Dialect::Postgres)
            .0
    }

    /// The bind parameters [`debug_sql`](Self::debug_sql) would send, in order.
    pub fn debug_params(&self) -> Vec<Value> {
        self.compile_select_with_order("*", true, Dialect::Postgres)
            .1
    }

    /// Runs `EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)` on the current query and
    /// returns the planner/execution output as a single `String` (rows joined by
    /// newlines).
    ///
    /// This is Postgres-only (SQLite's EXPLAIN is different); SQLite returns an
    /// [`OrmError::UnsupportedOnDialect`].
    pub async fn explain<E: djangors_db::DbExecutor>(&self, mut db: E) -> Result<String, OrmError> {
        let dialect = db.dialect();
        if dialect != Dialect::Postgres {
            return Err(OrmError::UnsupportedOnDialect(
                "explain() is Postgres-only; use debug_sql() on SQLite".to_string(),
            ));
        }
        let (select_sql, params) = self.compile_select();
        let explain_sql = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {select_sql}");
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();
        let mut conn = db.conn();
        let rows = conn.fetch_all(&explain_sql, &bind_values).await?;
        let lines: Vec<String> = rows
            .iter()
            .filter_map(|row| row.try_string(0).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(lines.join("\n"))
    }

    /// Excludes rows matching `expr` — the negation of [`filter`](Self::filter).
    pub fn exclude(mut self, expr: impl Into<UnresolvedExpr>) -> Result<Self, OrmError> {
        let resolved = Self::resolve_expr(expr.into())?;
        self.filters.push(Expr::Not(Box::new(resolved)));
        Ok(self)
    }

    /// Validates that `field` names a real field or relation on `T`.
    fn check_field(field: &'static str) -> Result<&'static str, OrmError> {
        Self::check_field_meta(T::meta(), field)
    }

    /// Rejects a filter, ordering or search on an encrypted field of `meta`'s model.
    /// Ciphertext is randomized, so it cannot be compared or sorted.
    fn reject_encrypted(meta: &'static ModelMeta, field: &str) -> Result<(), OrmError> {
        let encrypted = T::encrypted_fields().contains(&field);
        if encrypted && meta.struct_name == T::meta().struct_name {
            return Err(OrmError::InvalidQuery(format!(
                "cannot filter on encrypted field {field}"
            )));
        }
        Ok(())
    }

    /// Rejects using an encrypted field where SQL would have to read its plaintext
    /// (grouping, ordering by value, `MIN`/`MAX`/`SUM`/`AVG`, string functions).
    /// Every encrypted value has a random nonce, so the database only sees noise.
    fn reject_encrypted_use(field: &str, what: &str) -> Result<(), OrmError> {
        if T::encrypted_fields().contains(&field) {
            return Err(OrmError::InvalidQuery(format!(
                "cannot {what} encrypted field {field}"
            )));
        }
        Ok(())
    }

    /// Rejects aggregates other than `COUNT` on encrypted fields.
    fn reject_encrypted_agg(agg: &crate::aggregate::AggExpr) -> Result<(), OrmError> {
        if matches!(agg, crate::aggregate::AggExpr::Count { .. }) {
            return Ok(());
        }
        Self::reject_encrypted_use(agg_field(agg), "aggregate")
    }

    /// Decrypts the values of encrypted fields in a `values()` row.
    fn decrypt_value(field: &str, value: Value) -> Result<Value, OrmError> {
        if !T::encrypted_fields().contains(&field) {
            return Ok(value);
        }
        match value {
            Value::Text(stored) => Ok(Value::Text(crate::encryption::decrypt_str(&stored)?)),
            other => Ok(other),
        }
    }

    fn check_field_meta(
        meta: &'static ModelMeta,
        field: &'static str,
    ) -> Result<&'static str, OrmError> {
        let exists = meta.fields.iter().any(|f| f.name == field)
            || meta.relations.iter().any(|r| r.field_name == field);
        if !exists {
            return Err(OrmError::FieldNotFound {
                field: field.to_string(),
                model: meta.struct_name,
            });
        }
        Ok(field)
    }

    fn check_field_column_meta(
        meta: &'static ModelMeta,
        field: &str,
    ) -> Result<&'static str, OrmError> {
        if let Some(f) = meta
            .fields
            .iter()
            .find(|f| f.name == field || f.column_name == field)
        {
            Ok(f.column_name)
        } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field) {
            Ok(r.field_name)
        } else {
            Err(OrmError::FieldNotFound {
                field: field.to_string(),
                model: meta.struct_name,
            })
        }
    }

    /// Recursively resolves an [`UnresolvedExpr`] tree into an [`Expr`] tree.
    fn resolve_expr(expr: UnresolvedExpr) -> Result<Expr, OrmError> {
        Self::resolve_expr_with_meta(expr, T::meta(), None)
    }

    fn resolve_expr_with_meta(
        expr: UnresolvedExpr,
        meta: &'static ModelMeta,
        outer_meta: Option<&'static ModelMeta>,
    ) -> Result<Expr, OrmError> {
        match expr {
            UnresolvedExpr::And(compares) => {
                let mut resolved = Vec::new();
                for comp in compares {
                    let (field_name, suffix) = crate::expr::split_field_lookup(comp.field);
                    let field = Self::check_field_meta(meta, field_name)?;
                    Self::reject_encrypted(meta, field_name)?;
                    resolved.push(Expr::Compare {
                        field,
                        op: suffix_to_op(suffix),
                        value: comp.value,
                    });
                }
                Ok(Expr::And(resolved))
            }
            UnresolvedExpr::AndFields(compares) => {
                let mut resolved = Vec::new();
                for comp in compares {
                    let (left_name, suffix) = crate::expr::split_field_lookup(comp.left);
                    let left = Self::check_field_meta(meta, left_name)?;
                    let right = Self::check_field_meta(meta, comp.right)?;
                    Self::reject_encrypted(meta, left_name)?;
                    Self::reject_encrypted(meta, right)?;
                    resolved.push(Expr::CompareField {
                        left,
                        op: suffix_to_op(suffix),
                        right,
                    });
                }
                Ok(Expr::And(resolved))
            }
            UnresolvedExpr::AndOuter(compares) => {
                let outer_m = outer_meta.ok_or_else(|| {
                    OrmError::InvalidQuery(
                        "OuterRef can only be used within a subquery".to_string(),
                    )
                })?;
                let mut resolved = Vec::new();
                for comp in compares {
                    let (field_name, suffix) = crate::expr::split_field_lookup(comp.field);
                    let sub_col = Self::check_field_column_meta(meta, field_name)?;
                    let outer_col = Self::check_field_column_meta(outer_m, comp.outer_field)?;
                    Self::reject_encrypted(meta, field_name)?;
                    Self::reject_encrypted(outer_m, comp.outer_field)?;
                    resolved.push(Expr::OuterCompare {
                        field: sub_col,
                        op: suffix_to_op(suffix),
                        outer_field: outer_col,
                    });
                }
                Ok(Expr::And(resolved))
            }
            UnresolvedExpr::All(nodes) => {
                let mut resolved = Vec::new();
                for node in nodes {
                    resolved.push(Self::resolve_expr_with_meta(node, meta, outer_meta)?);
                }
                Ok(Expr::And(resolved))
            }
            UnresolvedExpr::Any(nodes) => {
                let mut resolved = Vec::new();
                for node in nodes {
                    resolved.push(Self::resolve_expr_with_meta(node, meta, outer_meta)?);
                }
                Ok(Expr::Or(resolved))
            }
            UnresolvedExpr::Negate(inner) => Ok(Expr::Not(Box::new(Self::resolve_expr_with_meta(
                *inner, meta, outer_meta,
            )?))),
            UnresolvedExpr::Exists {
                negated,
                subquery_meta,
                soft_delete_col,
                filters,
            } => {
                let sub_meta = subquery_meta();
                let mut resolved_filters = Vec::new();
                for f in filters {
                    resolved_filters.push(Self::resolve_expr_with_meta(f, sub_meta, Some(meta))?);
                }
                // Soft-deleted rows of the subquery model do not count as matches.
                // The subquery compiles its fields as raw `"table"."column"`, so the
                // column name is passed here.
                if let Some(col) = soft_delete_col {
                    resolved_filters.push(Expr::Compare {
                        field: col,
                        op: CompareOp::IsNull,
                        value: Value::Bool(true),
                    });
                }
                Ok(Expr::Exists {
                    negated,
                    table: sub_meta.table_name,
                    filters: resolved_filters,
                })
            }
        }
    }

    /// Groups by `group_by` and computes `aggs` per group — Django's
    /// `.values(...).annotate(...)`.
    pub async fn annotate<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        group_by: &[&'static str],
        aggs: Vec<(&'static str, crate::aggregate::AggExpr)>,
    ) -> Result<Vec<GroupRow>, OrmError> {
        if self.for_update.is_some() {
            return Err(OrmError::InvalidQuery(
                "FOR UPDATE cannot be used with annotate/group-by queries".to_string(),
            ));
        }
        let dialect = db.dialect();
        let meta = T::meta();

        if group_by.is_empty() {
            return Err(OrmError::InvalidQuery(
                "annotate requires at least one group-by field".to_string(),
            ));
        }

        for field in group_by {
            Self::check_field(field)?;
            Self::reject_encrypted_use(field, "group by")?;
        }
        for (_, agg) in &aggs {
            let field_name = agg_field(agg);
            if field_name != "*" {
                Self::check_field_named(field_name)?;
            }
            Self::reject_encrypted_agg(agg)?;
        }

        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                format!("\"{}\"", f.column_name)
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                format!("\"{}\"", r.field_name)
            } else {
                field_name.to_string()
            }
        };

        let group_cols: Vec<String> = group_by.iter().map(|f| field_to_col(f)).collect();

        let mut select_parts = group_cols.clone();
        for (_, agg) in &aggs {
            select_parts.push(agg_sql(agg, &field_to_col, dialect));
        }

        let mut clean_qs = self.clone();
        clean_qs.limit = None;
        clean_qs.offset = None;
        let (mut sql, params) =
            clean_qs.compile_select_with_order(&select_parts.join(", "), false, dialect);
        sql.push_str(&format!(" GROUP BY {}", group_cols.join(", ")));

        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();
        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut keys = Vec::with_capacity(group_by.len());
            for (i, field) in group_by.iter().enumerate() {
                keys.push((*field, decode_value(row, i, field_kind_of(field, meta))));
            }

            let mut aggregates = Vec::with_capacity(aggs.len());
            for (i, (alias, agg)) in aggs.iter().enumerate() {
                let idx = group_by.len() + i;
                let res = match agg {
                    crate::aggregate::AggExpr::Count { .. } => row
                        .try_i64(idx)
                        .ok()
                        .flatten()
                        .map(crate::aggregate::AggResult::I64)
                        .unwrap_or(crate::aggregate::AggResult::Null),
                    _ => row
                        .try_f64(idx)
                        .ok()
                        .flatten()
                        .map(crate::aggregate::AggResult::F64)
                        .unwrap_or(crate::aggregate::AggResult::Null),
                };
                aggregates.push(((*alias).to_string(), res));
            }

            out.push(GroupRow { keys, aggregates });
        }

        Ok(out)
    }

    /// Apply scalar functions and return as [`GroupRow`]s. Like [`annotate`](Self::annotate)
    /// but for scalar function expressions instead of aggregate functions.
    pub async fn annotate_funcs<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        group_by: &[&'static str],
        funcs: Vec<(&'static str, crate::aggregate::FuncExpr)>,
    ) -> Result<Vec<GroupRow>, OrmError> {
        if self.for_update.is_some() {
            return Err(OrmError::InvalidQuery(
                "FOR UPDATE cannot be used with annotate/group-by queries".to_string(),
            ));
        }
        let dialect = db.dialect();
        let meta = T::meta();

        if group_by.is_empty() {
            return Err(OrmError::InvalidQuery(
                "annotate_funcs requires at least one group-by field".to_string(),
            ));
        }

        for field in group_by {
            Self::check_field(field)?;
            Self::reject_encrypted_use(field, "group by")?;
        }
        for (_, func) in &funcs {
            for field in func_fields(func) {
                Self::reject_encrypted_use(field, "apply a SQL function to")?;
            }
        }

        let group_cols: Vec<String> = group_by
            .iter()
            .map(|f| {
                if let Some(fm) = meta.fields.iter().find(|m| m.name == *f) {
                    format!("\"{}\"", fm.column_name)
                } else {
                    format!("\"{}\"", f)
                }
            })
            .collect();

        let mut func_selects = Vec::new();
        let mut func_aliases = Vec::new();
        for (alias, func) in &funcs {
            let expr_sql = func.to_sql(&dialect);
            let col_alias = func.alias();
            func_selects.push(format!("{expr_sql} AS \"{col_alias}\""));
            func_aliases.push((alias.to_string(), col_alias));
        }

        let mut select_parts = group_cols.clone();
        select_parts.extend(func_selects);

        let mut clean_qs = self.clone();
        clean_qs.limit = None;
        clean_qs.offset = None;
        let (mut sql, params) =
            clean_qs.compile_select_with_order(&select_parts.join(", "), false, dialect);
        sql.push_str(&format!(" GROUP BY {}", group_cols.join(", ")));

        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();
        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;

        let mut results = Vec::new();
        for row in rows {
            let mut keys = Vec::new();
            let mut aggregates = Vec::new();
            for f in group_by {
                keys.push((
                    *f,
                    decode_value(
                        &row,
                        group_by.iter().position(|g| g == f).unwrap(),
                        field_kind_of(f, meta),
                    ),
                ));
            }
            for (alias, col_alias) in &func_aliases {
                let val = row_to_value(&row, col_alias)?;
                aggregates.push((alias.clone(), crate::aggregate::AggResult::from_value(&val)));
            }
            results.push(GroupRow { keys, aggregates });
        }
        Ok(results)
    }

    /// Selects only `fields`, returning raw column values — Django's `.values()`.
    pub async fn values<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        fields: &[&'static str],
    ) -> Result<Vec<Vec<(&'static str, Value)>>, OrmError> {
        self.check_for_update(&mut db)?;
        let dialect = db.dialect();
        let meta = T::meta();

        if fields.is_empty() {
            return Err(OrmError::InvalidQuery(
                "values requires at least one field".to_string(),
            ));
        }
        for field in fields {
            Self::check_field(field)?;
        }

        let select_list = fields
            .iter()
            .map(|f| {
                if let Some(fm) = meta.fields.iter().find(|m| m.name == *f) {
                    format!("\"{}\"", fm.column_name)
                } else {
                    format!("\"{}\"", f)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");

        let (sql, params) = self.compile_select_with_order(&select_list, true, dialect);
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();

        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;

        // Encrypted fields come back decrypted, as they do on a loaded model.
        rows.iter()
            .map(|row| {
                fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let value = decode_value(row, i, field_kind_of(f, meta));
                        Ok((*f, Self::decrypt_value(f, value)?))
                    })
                    .collect()
            })
            .collect()
    }

    /// Like [`values`](Self::values) but for a single field — Django's `.values_list(field, flat=True)`.
    pub async fn values_list<E: djangors_db::DbExecutor>(
        &self,
        db: E,
        field: &'static str,
    ) -> Result<Vec<Value>, OrmError> {
        let rows = self.values(db, &[field]).await?;
        Ok(rows
            .into_iter()
            .filter_map(|mut r| r.pop().map(|(_, v)| v))
            .collect())
    }

    /// Field-name validation for a non-`'static` name.
    fn check_field_named(field: &str) -> Result<(), OrmError> {
        let meta = T::meta();
        let exists = meta.fields.iter().any(|f| f.name == field)
            || meta.relations.iter().any(|r| r.field_name == field);
        if !exists {
            return Err(OrmError::FieldNotFound {
                field: field.to_string(),
                model: meta.struct_name,
            });
        }
        Ok(())
    }

    /// ILIKE-matches `term` against every field in `fields`, OR'd together.
    pub fn filter_or_icontains(
        mut self,
        fields: &[&'static str],
        term: &str,
    ) -> Result<Self, OrmError> {
        let meta = T::meta();
        let mut compares = Vec::new();
        for field_name in fields {
            if !meta.fields.iter().any(|f| f.name == *field_name) {
                return Err(OrmError::FieldNotFound {
                    field: field_name.to_string(),
                    model: meta.struct_name,
                });
            }
            Self::reject_encrypted(meta, field_name)?;
            compares.push(Expr::Compare {
                field: field_name,
                op: CompareOp::IContains,
                value: Value::Text(term.to_string()),
            });
        }
        self.filters.push(Expr::Or(compares));
        Ok(self)
    }

    /// Performs a full-text search on the given text fields across both backends.
    ///
    /// - **PostgreSQL**: uses `to_tsvector('english', ...) @@ plainto_tsquery('english', query)`.
    /// - **SQLite / MySQL**: performs multi-term case-insensitive `LIKE` matching across the
    ///   specified fields (all whitespace-separated terms must match at least one field).
    pub fn search(mut self, query: &str, fields: &[&'static str]) -> Result<Self, OrmError> {
        let meta = T::meta();
        for field in fields {
            if !meta.fields.iter().any(|f| f.name == *field) {
                return Err(OrmError::FieldNotFound {
                    field: (*field).to_string(),
                    model: meta.struct_name,
                });
            }
            Self::reject_encrypted(meta, field)?;
        }
        self.search_query = Some((fields.to_vec(), query.to_string()));
        Ok(self)
    }

    /// Filters rows matching an FTS5 virtual table query on SQLite (`rowid IN (SELECT rowid FROM {fts_table} WHERE {fts_table} MATCH ?)`).
    ///
    /// Create the virtual table via [`create_fts5_table`] and populate it with [`index_fts5_row`].
    pub fn search_fts5(mut self, fts_table: &str, query: &str) -> Result<Self, OrmError> {
        if !fts_table
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || fts_table.is_empty()
        {
            return Err(OrmError::InvalidQuery(format!(
                "invalid FTS5 table name '{fts_table}'"
            )));
        }
        self.fts5_query = Some((fts_table.to_string(), query.to_string()));
        Ok(self)
    }

    /// Orders results by similarity to `query` using the requested [`VectorDistanceMetric`].
    ///
    /// - **PostgreSQL**: generates `ORDER BY "col" <=> '[...]'::vector` (or `<->`, `<#>`).
    /// - **SQLite**: computes exact distances in Rust and sorts before applying `offset`/`limit`.
    pub fn order_by_vector_distance(
        mut self,
        field: &str,
        query: &crate::vector::Vector,
        metric: crate::vector::VectorDistanceMetric,
    ) -> Result<Self, OrmError> {
        let meta = T::meta();
        let f = meta
            .fields
            .iter()
            .find(|f| f.name == field)
            .ok_or_else(|| OrmError::FieldNotFound {
                field: field.to_string(),
                model: meta.struct_name,
            })?;
        self.vector_order = Some((f.column_name.to_string(), metric, query.clone()));
        Ok(self)
    }

    /// Orders results by ascending cosine distance (`<=>` in pgvector) to `query`.
    pub fn order_by_cosine_distance(
        self,
        field: &str,
        query: &crate::vector::Vector,
    ) -> Result<Self, OrmError> {
        self.order_by_vector_distance(field, query, crate::vector::VectorDistanceMetric::Cosine)
    }

    /// Orders results by ascending Euclidean L2 distance (`<->` in pgvector) to `query`.
    pub fn order_by_l2_distance(
        self,
        field: &str,
        query: &crate::vector::Vector,
    ) -> Result<Self, OrmError> {
        self.order_by_vector_distance(field, query, crate::vector::VectorDistanceMetric::L2)
    }

    /// Orders results by ascending negative inner product (`<#>` in pgvector) to `query`.
    pub fn order_by_inner_product(
        self,
        field: &str,
        query: &crate::vector::Vector,
    ) -> Result<Self, OrmError> {
        self.order_by_vector_distance(
            field,
            query,
            crate::vector::VectorDistanceMetric::InnerProduct,
        )
    }

    /// Restricts to rows where `field` is in `[gte, lt)`.
    pub fn filter_datetime_range(
        mut self,
        field: &'static str,
        gte: chrono::DateTime<chrono::Utc>,
        lt: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, OrmError> {
        let meta = T::meta();
        if !meta.fields.iter().any(|f| f.name == field) {
            return Err(OrmError::FieldNotFound {
                field: field.to_string(),
                model: meta.struct_name,
            });
        }
        Self::reject_encrypted(meta, field)?;
        self.filters.push(Expr::And(vec![
            Expr::Compare {
                field,
                op: CompareOp::Gte,
                value: Value::DateTime(gte),
            },
            Expr::Compare {
                field,
                op: CompareOp::Lt,
                value: Value::DateTime(lt),
            },
        ]));
        Ok(self)
    }

    /// Restricts results to rows strictly after a keyset position.
    pub fn after(
        mut self,
        order_field: &'static str,
        order_value: Value,
        pk_field: &'static str,
        cursor_pk: i64,
        descending: bool,
    ) -> Result<Self, OrmError> {
        let meta = T::meta();
        for field in [order_field, pk_field] {
            if !meta.fields.iter().any(|f| f.name == field)
                && !meta.relations.iter().any(|r| r.field_name == field)
            {
                return Err(OrmError::FieldNotFound {
                    field: field.to_string(),
                    model: meta.struct_name,
                });
            }
            Self::reject_encrypted(meta, field)?;
        }
        let op = if descending {
            CompareOp::Lt
        } else {
            CompareOp::Gt
        };
        self.filters.push(Expr::Or(vec![
            Expr::Compare {
                field: order_field,
                op,
                value: order_value.clone(),
            },
            Expr::And(vec![
                Expr::Compare {
                    field: order_field,
                    op: CompareOp::Eq,
                    value: order_value,
                },
                Expr::Compare {
                    field: pk_field,
                    op,
                    value: Value::I64(cursor_pk),
                },
            ]),
        ]));
        Ok(self)
    }

    /// Order results by the given field. A leading `-` means descending.
    pub fn order_by(mut self, field: &str) -> Result<Self, OrmError> {
        let (clean_field, desc) = if let Some(stripped) = field.strip_prefix('-') {
            (stripped, true)
        } else {
            (field, false)
        };
        let meta = T::meta();
        Self::reject_encrypted(meta, clean_field)?;
        let col = if let Some(f) = meta.fields.iter().find(|f| f.name == clean_field) {
            format!("\"{}\"", f.column_name)
        } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == clean_field) {
            format!("\"{}\"", r.field_name)
        } else {
            return Err(OrmError::FieldNotFound {
                field: clean_field.to_string(),
                model: meta.struct_name,
            });
        };
        self.order_by.push((col, desc));
        Ok(self)
    }

    /// Restricts the maximum number of rows returned by the query.
    pub fn limit(mut self, n: i64) -> Self {
        self.limit = Some(n);
        self
    }

    /// Sets the number of rows to skip before starting to return rows.
    pub fn offset(mut self, n: i64) -> Self {
        self.offset = Some(n);
        self
    }

    /// Locks the selected rows for the remainder of the transaction.
    pub fn select_for_update(mut self) -> Self {
        if self.for_update.is_none() {
            self.for_update = Some(ForUpdate::default());
        }
        self
    }

    /// Return immediately with an error instead of waiting for a lock.
    /// Mutually exclusive with `skip_locked`.
    pub fn nowait(mut self) -> Self {
        let mut fu = self.for_update.unwrap_or_default();
        fu.nowait = true;
        fu.skip_locked = false;
        self.for_update = Some(fu);
        self
    }

    /// Skip rows that are already locked rather than waiting.
    /// Mutually exclusive with `nowait`.
    pub fn skip_locked(mut self) -> Self {
        let mut fu = self.for_update.unwrap_or_default();
        fu.skip_locked = true;
        fu.nowait = false;
        self.for_update = Some(fu);
        self
    }

    /// Restrict locking to the named related tables (Postgres `FOR UPDATE OF`).
    pub fn lock_of(mut self, tables: &[&str]) -> Self {
        let mut fu = self.for_update.unwrap_or_default();
        fu.of = tables.iter().map(|s| s.to_string()).collect();
        self.for_update = Some(fu);
        self
    }

    /// Validates `for_update` requirements against the execution target.
    pub(crate) fn check_for_update<E: djangors_db::DbExecutor>(
        &self,
        db: &mut E,
    ) -> Result<(), OrmError> {
        if let Some(ref fu) = self.for_update {
            if !db.conn().in_transaction() {
                return Err(OrmError::SelectForUpdateOutsideTransaction);
            }
            if db.dialect() == Dialect::Sqlite && (fu.nowait || fu.skip_locked || !fu.of.is_empty())
            {
                return Err(OrmError::UnsupportedOnDialect(
                    "nowait, skip_locked, and lock_of are not supported on SQLite".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Compiles the query to a `SELECT *` SQL string and parameter list.
    pub fn compile_select(&self) -> (String, Vec<Value>) {
        self.compile_select_custom("*")
    }

    /// Compiles the query to a SQL string selecting `select_list` columns and parameter list.
    pub fn compile_select_custom(&self, select_list: &str) -> (String, Vec<Value>) {
        self.compile_select_with_order(select_list, true, Dialect::Postgres)
    }

    /// The body of the `WHERE` clause (without the keyword): the filter expressions,
    /// plus `"deleted_at" IS NULL` for soft-delete models unless
    /// [`with_deleted`](Self::with_deleted) was called. Returns `None` when there is
    /// nothing to filter on. Bind values are appended to `params`.
    fn where_body(
        &self,
        field_to_col: &dyn Fn(&str) -> String,
        params: &mut Vec<Value>,
        param_idx: &mut usize,
        dialect: Dialect,
    ) -> Option<String> {
        let meta = T::meta();
        let mut parts = Vec::new();
        if self.is_none {
            return Some("FALSE".to_string());
        }
        if !self.filters.is_empty() {
            let combined = Expr::And(self.filters.clone());
            parts.push(compile_expr_sql(
                &combined,
                meta.table_name,
                None,
                field_to_col,
                params,
                param_idx,
                dialect,
            ));
        }
        if !self.include_deleted {
            if let Some(col) = T::soft_delete_column() {
                parts.push(format!("\"{}\" IS NULL", col));
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" AND "))
        }
    }

    /// Compiles the query with specified dialect.
    pub(crate) fn compile_select_with_order(
        &self,
        select_list: &str,
        include_order: bool,
        dialect: Dialect,
    ) -> (String, Vec<Value>) {
        let meta = T::meta();
        let effective_select: String = if select_list == "*"
            && (!self.only_fields.is_empty() || !self.defer_fields.is_empty())
        {
            let pk_name = meta.fields.iter().find(|f| f.primary_key).map(|f| f.name);
            let cols: Vec<String> = meta
                .fields
                .iter()
                .filter(|f| {
                    if Some(f.name) == pk_name {
                        return true;
                    }
                    if !self.only_fields.is_empty() {
                        self.only_fields.iter().any(|o| o == f.name)
                    } else {
                        !self.defer_fields.iter().any(|d| d == f.name)
                    }
                })
                .map(|f| format!("\"{}\"", f.column_name))
                .collect();
            if cols.is_empty() {
                "*".to_string()
            } else {
                cols.join(", ")
            }
        } else {
            select_list.to_string()
        };
        let distinct_prefix = if !self.distinct_on.is_empty() && dialect == Dialect::Postgres {
            format!("DISTINCT ON ({}) ", self.distinct_on.join(", "))
        } else if self.distinct {
            "DISTINCT ".to_string()
        } else {
            String::new()
        };
        let mut sql = format!(
            "SELECT {}{} FROM \"{}\"",
            distinct_prefix, effective_select, meta.table_name
        );
        let mut params = Vec::new();
        let mut param_idx = 1;

        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                format!("\"{}\"", f.column_name)
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                format!("\"{}\"", r.field_name)
            } else {
                field_name.to_string()
            }
        };

        if let Some(where_clause) =
            self.where_body(&field_to_col, &mut params, &mut param_idx, dialect)
        {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clause);
        }

        if let Some((ref fields, ref query)) = self.search_query {
            let escaped = query.replace('\'', "''");
            let sc = match dialect {
                Dialect::Postgres => {
                    let tsvector_parts: Vec<String> = fields
                        .iter()
                        .map(|f| format!("coalesce(\"{}\"::text, '')", f))
                        .collect();
                    format!(
                        "to_tsvector('english', {}) @@ plainto_tsquery('english', '{escaped}')",
                        tsvector_parts.join(" || ' ' || ")
                    )
                }
                Dialect::Sqlite | Dialect::MySql => {
                    let terms: Vec<&str> = query.split_whitespace().collect();
                    if terms.is_empty() || fields.is_empty() {
                        "1 = 1".to_string()
                    } else {
                        let mut term_clauses = Vec::with_capacity(terms.len());
                        for term in terms {
                            let pat = term
                                .replace('\'', "''")
                                .replace('%', "\\%")
                                .replace('_', "\\_");
                            let field_ors: Vec<String> = fields
                                .iter()
                                .map(|f| {
                                    if dialect == Dialect::Sqlite {
                                        format!("LOWER(COALESCE(\"{f}\", '')) LIKE LOWER('%{pat}%') ESCAPE '\\'")
                                    } else {
                                        format!("LOWER(COALESCE(`{f}`, '')) LIKE LOWER('%{pat}%')")
                                    }
                                })
                                .collect();
                            term_clauses.push(format!("({})", field_ors.join(" OR ")));
                        }
                        term_clauses.join(" AND ")
                    }
                }
            };
            if sql.contains(" WHERE ") {
                sql.push_str(&format!(" AND ({sc})"));
            } else {
                sql.push_str(&format!(" WHERE ({sc})"));
            }
        }

        if let Some((ref fts_table, ref fts_q)) = self.fts5_query {
            let escaped = fts_q.replace('\'', "''");
            let clause = match dialect {
                Dialect::Sqlite => format!(
                    "rowid IN (SELECT rowid FROM \"{fts_table}\" WHERE \"{fts_table}\" MATCH '{escaped}')"
                ),
                Dialect::Postgres | Dialect::MySql => {
                    // Fallback when FTS5 helper is invoked against Postgres/MySQL.
                    let _ = fts_table;
                    "1 = 1".to_string()
                }
            };
            if sql.contains(" WHERE ") {
                sql.push_str(&format!(" AND ({clause})"));
            } else {
                sql.push_str(&format!(" WHERE ({clause})"));
            }
        }

        let order_source = if !include_order {
            Vec::new()
        } else if !self.order_by.is_empty() {
            self.order_by.clone()
        } else {
            meta.ordering
                .iter()
                .map(|&field| {
                    let (clean_field, desc) = if let Some(stripped) = field.strip_prefix('-') {
                        (stripped, true)
                    } else {
                        (field, false)
                    };
                    let col = if let Some(f) = meta.fields.iter().find(|f| f.name == clean_field) {
                        format!("\"{}\"", f.column_name)
                    } else if let Some(r) =
                        meta.relations.iter().find(|r| r.field_name == clean_field)
                    {
                        format!("\"{}\"", r.field_name)
                    } else {
                        clean_field.to_string()
                    };
                    (col, desc)
                })
                .collect()
        };

        if !order_source.is_empty() {
            let order_parts: Vec<String> = order_source
                .iter()
                .map(|(col, desc)| {
                    if *desc {
                        format!("{} DESC", col)
                    } else {
                        format!("{} ASC", col)
                    }
                })
                .collect();
            sql.push_str(" ORDER BY ");
            sql.push_str(&order_parts.join(", "));
        } else if include_order && dialect == Dialect::Postgres {
            if let Some((ref col, metric, ref query_vec)) = self.vector_order {
                let lit = query_vec.to_pgvector_literal();
                sql.push_str(&format!(
                    " ORDER BY \"{}\" {} '{}'::vector ASC",
                    col,
                    metric.pgvector_operator(),
                    lit
                ));
            }
        }

        let defer_limit_for_sqlite_vec =
            dialect != Dialect::Postgres && self.vector_order.is_some();

        if !defer_limit_for_sqlite_vec {
            if let Some(limit) = self.limit {
                sql.push_str(&format!(" LIMIT {}", limit));
            }
            if let Some(offset) = self.offset {
                if self.limit.is_none() && dialect == Dialect::Sqlite {
                    sql.push_str(" LIMIT -1");
                }
                sql.push_str(&format!(" OFFSET {}", offset));
            }
        }

        if dialect == Dialect::Postgres {
            if let Some(ref fu) = self.for_update {
                sql.push_str(" FOR UPDATE");
                if !fu.of.is_empty() {
                    let quoted_tables: Vec<String> =
                        fu.of.iter().map(|t| dialect.quote_ident(t)).collect();
                    sql.push_str(" OF ");
                    sql.push_str(&quoted_tables.join(", "));
                }
                if fu.nowait {
                    sql.push_str(" NOWAIT");
                } else if fu.skip_locked {
                    sql.push_str(" SKIP LOCKED");
                }
            }
        }

        if !self.set_ops.is_empty() {
            let mut combined = sql;
            for (op, other) in &self.set_ops {
                let op_kw = match op {
                    SetOp::Union => "UNION",
                    SetOp::UnionAll => "UNION ALL",
                    SetOp::Intersection => "INTERSECT",
                    SetOp::Difference => "EXCEPT",
                };
                let (other_sql, other_params) =
                    other.compile_select_with_order(select_list, false, dialect);
                // Re-index Postgres placeholders for the right-hand side.
                let adjusted_other = if dialect == Dialect::Postgres && !other_params.is_empty() {
                    let offset = params.len();
                    let mut out = other_sql;
                    for i in (1..=other_params.len()).rev() {
                        out = out.replace(&format!("${i}"), &format!("${}", i + offset));
                    }
                    out
                } else {
                    other_sql
                };
                params.extend(other_params);
                combined = format!("{combined} {op_kw} {adjusted_other}");
            }
            sql = combined;
        }

        (sql, params)
    }

    /// Returns the earliest object ordered ascending by `field` (Django's `QuerySet.earliest()`).
    pub async fn earliest<E: djangors_db::DbExecutor>(
        self,
        field: &str,
        db: E,
    ) -> Result<Option<T>, OrmError> {
        let clean = field.strip_prefix('-').unwrap_or(field);
        self.order_by(clean)?.first(db).await
    }

    /// Returns the latest object ordered descending by `field` (Django's `QuerySet.latest()`).
    pub async fn latest<E: djangors_db::DbExecutor>(
        self,
        field: &str,
        db: E,
    ) -> Result<Option<T>, OrmError> {
        let desc_field = if field.starts_with('-') {
            field.to_string()
        } else {
            format!("-{field}")
        };
        self.order_by(&desc_field)?.first(db).await
    }

    /// Fetches all matching rows into a map keyed by primary key string (Django's `QuerySet.in_bulk()`).
    pub async fn in_bulk<E: djangors_db::DbExecutor>(
        &self,
        db: E,
    ) -> Result<std::collections::HashMap<String, T>, OrmError> {
        let pk_name = T::meta()
            .fields
            .iter()
            .find(|f| f.primary_key)
            .map(|f| f.name)
            .unwrap_or("id");
        let rows = self.all(db).await?;
        let mut map = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            if let Some((_, val)) = row.field_values().into_iter().find(|(k, _)| *k == pk_name) {
                map.insert(val.to_string(), row);
            }
        }
        Ok(map)
    }

    /// Executes a raw SQL query and maps each row into `T` (Django's `Model.objects.raw()`).
    pub async fn raw<E: djangors_db::DbExecutor>(
        mut db: E,
        sql: &str,
        params: &[Value],
    ) -> Result<Vec<T>, OrmError> {
        db.record_query();
        let bind_params: Vec<BindValue> = params.iter().cloned().map(BindValue::from).collect();
        let rows = db.conn().fetch_all(sql, &bind_params).await?;
        rows.iter().map(T::from_row).collect()
    }

    /// Updates `fields` on each model instance in `objs` by primary key (Django's `QuerySet.bulk_update()`).
    pub async fn bulk_update<E: djangors_db::DbExecutor>(
        mut db: E,
        objs: &[T],
        fields: &[&str],
    ) -> Result<u64, OrmError> {
        if objs.is_empty() || fields.is_empty() {
            return Ok(0);
        }
        let meta = T::meta();
        let pk_field = meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .ok_or_else(|| OrmError::InvalidQuery("model has no primary key".to_string()))?;
        for &field in fields {
            Self::check_field_column_meta(meta, field)?;
        }
        let dialect = db.dialect();
        let mut total_affected = 0u64;
        for obj in objs {
            let fvals = obj.field_values();
            let pk_val = fvals
                .iter()
                .find(|(k, _)| *k == pk_field.name)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| OrmError::InvalidQuery("missing primary key value".to_string()))?;
            let mut set_clauses = Vec::with_capacity(fields.len());
            let mut bind_params = Vec::with_capacity(fields.len() + 1);
            for (idx, &field) in fields.iter().enumerate() {
                let col = Self::check_field_column_meta(meta, field)?;
                let val = fvals
                    .iter()
                    .find(|(k, _)| *k == field)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null);
                set_clauses.push(format!(
                    "{} = {}",
                    dialect.quote_ident(col),
                    dialect.placeholder(idx + 1)
                ));
                bind_params.push(BindValue::from(val));
            }
            let pk_placeholder = dialect.placeholder(fields.len() + 1);
            bind_params.push(BindValue::from(pk_val));
            let sql = format!(
                "UPDATE {} SET {} WHERE {} = {}",
                dialect.quote_ident(meta.table_name),
                set_clauses.join(", "),
                dialect.quote_ident(pk_field.column_name),
                pk_placeholder
            );
            db.record_query();
            let affected = db.conn().execute(&sql, &bind_params).await?;
            total_affected += affected;
        }
        crate::query_cache::invalidate_table(meta.table_name);
        Ok(total_affected)
    }

    /// Streams matching rows in chunks of `chunk_size` to bound memory usage
    /// (Django's `QuerySet.iterator(chunk_size=...)`).
    pub async fn iterator<E, F>(
        &self,
        db: &djangors_db::Database,
        chunk_size: i64,
        mut on_chunk: F,
    ) -> Result<usize, OrmError>
    where
        F: FnMut(Vec<T>) -> Result<(), E>,
        E: std::fmt::Display,
    {
        let step = chunk_size.max(1);
        let mut offset = self.offset.unwrap_or(0);
        let mut total = 0usize;
        loop {
            let mut page = self.clone();
            page.limit = Some(step);
            page.offset = Some(offset);
            let batch = page.all(db).await?;
            let n = batch.len();
            if n == 0 {
                break;
            }
            total += n;
            on_chunk(batch).map_err(|e| OrmError::InvalidQuery(e.to_string()))?;
            if (n as i64) < step {
                break;
            }
            offset += step;
        }
        Ok(total)
    }

    /// Executes the query and returns all matching model instances.
    pub async fn all<E: djangors_db::DbExecutor>(&self, mut db: E) -> Result<Vec<T>, OrmError> {
        self.check_for_update(&mut db)?;
        let dialect = db.dialect();
        let (sql, params) = self.compile_select_with_order("*", true, dialect);
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();
        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(T::from_row(&row)?);
        }
        if dialect != Dialect::Postgres {
            if let Some((ref col, metric, ref query_vec)) = self.vector_order {
                let field_name = T::meta()
                    .fields
                    .iter()
                    .find(|f| f.column_name == col)
                    .map(|f| f.name)
                    .unwrap_or(col.as_str());
                results.sort_by(|a, b| {
                    let dist = |item: &T| -> f32 {
                        item.field_values()
                            .into_iter()
                            .find(|(k, _)| *k == field_name)
                            .and_then(|(_, v)| match v {
                                Value::Text(s) => crate::vector::Vector::from_pgvector_literal(&s).ok(),
                                _ => None,
                            })
                            .map(|v| v.distance(query_vec, metric))
                            .unwrap_or(f32::INFINITY)
                    };
                    dist(a)
                        .partial_cmp(&dist(b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                if let Some(offset) = self.offset {
                    let start = (offset.max(0) as usize).min(results.len());
                    results = results.split_off(start);
                }
                if let Some(limit) = self.limit {
                    results.truncate(limit.max(0) as usize);
                }
            }
        }
        Ok(results)
    }

    /// Like [`all`](Self::all) but caches the result in-process.
    ///
    /// Requires prior [`cache`](Self::cache) to have been called and
    /// `T: Clone + Send + Sync + 'static`. When the cache is enabled, repeated
    /// calls with the same query return the cached rows without hitting the
    /// database until the TTL expires or the table is invalidated by a write.
    pub async fn cached_all<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
    ) -> Result<Vec<T>, OrmError>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.check_for_update(&mut db)?;
        let dialect = db.dialect();
        let (sql, params) = self.compile_select_with_order("*", true, dialect);
        let table = T::meta().table_name;

        if let Some(rows) = crate::query_cache::get_cached::<T>(table, &sql, &params) {
            return Ok(rows);
        }

        let bind_values: Vec<BindValue> = params.iter().cloned().map(BindValue::from).collect();
        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            results.push(T::from_row(&row)?);
        }
        if let Some(ttl) = self.cache_ttl {
            crate::query_cache::put_cached(table, &sql, &params, results.clone(), Some(ttl));
        }
        Ok(results)
    }

    /// Fetches a single model instance matching the query filters.
    pub async fn get<E: djangors_db::DbExecutor>(&self, db: E) -> Result<T, OrmError> {
        let mut cloned = self.clone();
        cloned.limit = Some(2);
        let results = cloned.all(db).await?;
        if results.is_empty() {
            Err(OrmError::NotFound {
                model: T::meta().struct_name,
            })
        } else if results.len() > 1 {
            Err(OrmError::MultipleObjectsReturned {
                model: T::meta().struct_name,
            })
        } else {
            Ok(results.into_iter().next().unwrap())
        }
    }

    /// Fetches the first model instance matching the query filters, or `None` if no row matches.
    pub async fn first<E: djangors_db::DbExecutor>(&self, db: E) -> Result<Option<T>, OrmError> {
        let mut cloned = self.clone();
        cloned.limit = Some(1);
        let results = cloned.all(db).await?;
        Ok(results.into_iter().next())
    }

    /// Returns `true` if at least one record matches the query filters.
    pub async fn exists<E: djangors_db::DbExecutor>(&self, mut db: E) -> Result<bool, OrmError> {
        self.check_for_update(&mut db)?;
        let mut cloned = self.clone();
        cloned.limit = Some(1);
        let dialect = db.dialect();
        let (sql, params) = cloned.compile_select_with_order("1", false, dialect);
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();
        db.record_query();
        let row_opt = db.conn().fetch_optional(&sql, &bind_values).await?;
        Ok(row_opt.is_some())
    }

    /// Returns the total count of rows matching the query filters.
    pub async fn count<E: djangors_db::DbExecutor>(&self, mut db: E) -> Result<i64, OrmError> {
        if self.for_update.is_some() {
            return Err(OrmError::InvalidQuery(
                "FOR UPDATE cannot be used with aggregate queries".to_string(),
            ));
        }
        let dialect = db.dialect();
        let (sql, params) = self.compile_select_with_order("COUNT(*)", false, dialect);
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();
        db.record_query();
        let row = db.conn().fetch_one(&sql, &bind_values).await?;
        let count = row.try_i64(0)?.unwrap_or(0);
        Ok(count)
    }

    /// Executes aggregate functions over matching rows.
    pub async fn aggregate<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        aggs: Vec<crate::aggregate::AggExpr>,
    ) -> Result<Vec<crate::aggregate::AggResult>, OrmError> {
        if self.for_update.is_some() {
            return Err(OrmError::InvalidQuery(
                "FOR UPDATE cannot be used with aggregate queries".to_string(),
            ));
        }
        let dialect = db.dialect();
        let meta = T::meta();

        for agg in &aggs {
            Self::reject_encrypted_agg(agg)?;
            let field_name = match agg {
                crate::aggregate::AggExpr::Count { field } => *field,
                crate::aggregate::AggExpr::Sum { field } => *field,
                crate::aggregate::AggExpr::Avg { field } => *field,
                crate::aggregate::AggExpr::Min { field } => *field,
                crate::aggregate::AggExpr::Max { field } => *field,
            };

            if field_name != "*" {
                let exists = meta.fields.iter().any(|f| f.name == field_name)
                    || meta.relations.iter().any(|r| r.field_name == field_name);
                if !exists {
                    return Err(OrmError::FieldNotFound {
                        field: field_name.to_string(),
                        model: meta.struct_name,
                    });
                }
            }
        }

        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                format!("\"{}\"", f.column_name)
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                format!("\"{}\"", r.field_name)
            } else {
                field_name.to_string()
            }
        };

        let mut agg_sql_parts = Vec::new();
        for agg in &aggs {
            let part = match agg {
                crate::aggregate::AggExpr::Count { field } => {
                    if *field == "*" {
                        "COUNT(*)".to_string()
                    } else {
                        format!("COUNT({})", field_to_col(field))
                    }
                }
                crate::aggregate::AggExpr::Sum { field } => {
                    dialect.cast_float(&format!("SUM({})", field_to_col(field)))
                }
                crate::aggregate::AggExpr::Avg { field } => {
                    dialect.cast_float(&format!("AVG({})", field_to_col(field)))
                }
                crate::aggregate::AggExpr::Min { field } => {
                    dialect.cast_float(&format!("MIN({})", field_to_col(field)))
                }
                crate::aggregate::AggExpr::Max { field } => {
                    dialect.cast_float(&format!("MAX({})", field_to_col(field)))
                }
            };
            agg_sql_parts.push(part);
        }

        let select_list = agg_sql_parts.join(", ");
        let mut clean_qs = self.clone();
        clean_qs.limit = None;
        clean_qs.offset = None;

        let (sql, params) = clean_qs.compile_select_with_order(&select_list, false, dialect);
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();

        db.record_query();
        let row = db.conn().fetch_one(&sql, &bind_values).await?;

        let mut results = Vec::with_capacity(aggs.len());
        for (i, agg) in aggs.iter().enumerate() {
            let res = match agg {
                crate::aggregate::AggExpr::Count { .. } => {
                    let count_val = row.try_i64(i)?.unwrap_or(0);
                    crate::aggregate::AggResult::I64(count_val)
                }
                _ => {
                    let val = row.try_f64(i)?;
                    match val {
                        Some(v) => crate::aggregate::AggResult::F64(v),
                        None => crate::aggregate::AggResult::Null,
                    }
                }
            };
            results.push(res);
        }

        Ok(results)
    }

    /// Executes a bulk UPDATE query matching current filters with specified field set expressions.
    pub async fn update<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        mut sets: Vec<(&'static str, SetExpr)>,
    ) -> Result<u64, OrmError> {
        let dialect = db.dialect();
        let meta = T::meta();
        for (field_name, set_expr) in sets.iter_mut() {
            if !T::encrypted_fields().contains(field_name) {
                continue;
            }
            match set_expr {
                SetExpr::Literal(value) => {
                    let plain = std::mem::replace(value, Value::Null);
                    *value = crate::encryption::encrypt_value(plain)?;
                }
                SetExpr::FieldOp { .. } => {
                    return Err(OrmError::InvalidQuery(format!(
                        "cannot apply arithmetic to encrypted field {field_name}"
                    )));
                }
            }
        }

        for &(field_name, ref set_expr) in &sets {
            let exists_lhs = meta.fields.iter().any(|f| f.name == field_name)
                || meta.relations.iter().any(|r| r.field_name == field_name);
            if !exists_lhs {
                return Err(OrmError::FieldNotFound {
                    field: field_name.to_string(),
                    model: meta.struct_name,
                });
            }

            if let SetExpr::FieldOp {
                field: rhs_field, ..
            } = set_expr
            {
                let exists_rhs = meta.fields.iter().any(|f| f.name == *rhs_field)
                    || meta.relations.iter().any(|r| r.field_name == *rhs_field);
                if !exists_rhs {
                    return Err(OrmError::FieldNotFound {
                        field: rhs_field.to_string(),
                        model: meta.struct_name,
                    });
                }
            }
        }

        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                format!("\"{}\"", f.column_name)
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                format!("\"{}\"", r.field_name)
            } else {
                field_name.to_string()
            }
        };

        let mut bind_values: Vec<BindValue> = Vec::new();
        let mut param_idx = 1;
        let mut set_parts = Vec::new();

        for &(field_name, ref set_expr) in &sets {
            let lhs_col = field_to_col(field_name);
            let nk = null_bind_kind_for(field_name, meta);
            match set_expr {
                SetExpr::Literal(v) => {
                    set_parts.push(format!("{} = {}", lhs_col, dialect.placeholder(param_idx)));
                    let bv = match v {
                        Value::Null => BindValue::Null(nk),
                        other => BindValue::from(other.clone()),
                    };
                    bind_values.push(bv);
                    param_idx += 1;
                }
                SetExpr::FieldOp { field, op, operand } => {
                    let rhs_col = field_to_col(field);
                    let op_sql = match op {
                        ArithOp::Add => "+",
                        ArithOp::Sub => "-",
                        ArithOp::Mul => "*",
                        ArithOp::Div => "/",
                    };
                    set_parts.push(format!(
                        "{} = {} {} {}",
                        lhs_col,
                        rhs_col,
                        op_sql,
                        dialect.placeholder(param_idx)
                    ));
                    let bv = match operand {
                        Value::Null => BindValue::Null(nk),
                        other => BindValue::from(other.clone()),
                    };
                    bind_values.push(bv);
                    param_idx += 1;
                }
            }
        }

        let mut raw_params = Vec::new();
        let mut sql = format!(
            "UPDATE \"{}\" SET {}",
            meta.table_name,
            set_parts.join(", ")
        );
        if let Some(where_clause) =
            self.where_body(&field_to_col, &mut raw_params, &mut param_idx, dialect)
        {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clause);
        }
        for p in raw_params {
            bind_values.push(BindValue::from(p));
        }

        db.record_query();
        let rows_affected = db.conn().execute(&sql, &bind_values).await?;
        Ok(rows_affected)
    }

    /// Deletes all rows matching the current filters — Django's `queryset.delete()`.
    ///
    /// Returns the number of rows affected. Unlike [`delete_by_pk`](Self::delete_by_pk),
    /// this operates on the whole filtered set. If no filter is applied, every row in
    /// the table is affected.
    ///
    /// On a `#[djangors(soft_delete)]` model this stamps `deleted_at` with the current
    /// time instead of removing rows (already soft-deleted rows are skipped). Use
    /// [`with_deleted`](Self::with_deleted) to hard-delete, or
    /// [`force_delete`](Self::force_delete) to hard-delete without it.
    pub async fn delete<E: djangors_db::DbExecutor>(&self, db: E) -> Result<u64, OrmError> {
        let soft = !self.include_deleted;
        self.delete_rows(db, soft).await
    }

    /// Permanently deletes all rows matching the current filters, even on a
    /// soft-delete model. Already soft-deleted rows are skipped unless
    /// [`with_deleted`](Self::with_deleted) or [`only_deleted`](Self::only_deleted) is set.
    pub async fn force_delete<E: djangors_db::DbExecutor>(&self, db: E) -> Result<u64, OrmError> {
        self.delete_rows(db, false).await
    }

    /// Shared implementation of [`delete`](Self::delete) and [`force_delete`](Self::force_delete).
    ///
    /// When `soft` is true and `T` is a soft-delete model, the statement is an
    /// `UPDATE` that sets `deleted_at` to now; otherwise it is a `DELETE`.
    async fn delete_rows<E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        soft: bool,
    ) -> Result<u64, OrmError> {
        let dialect = db.dialect();
        let meta = T::meta();

        let field_to_col = |field_name: &str| -> String {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                format!("\"{}\"", f.column_name)
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                format!("\"{}\"", r.field_name)
            } else {
                field_name.to_string()
            }
        };

        let soft_col = if soft { T::soft_delete_column() } else { None };
        let mut bind_values: Vec<BindValue> = Vec::new();
        let mut param_idx = 1;
        let mut sql = match soft_col {
            Some(col) => {
                bind_values.push(BindValue::from(Value::DateTime(chrono::Utc::now())));
                let set_sql = format!("\"{}\" = {}", col, dialect.placeholder(param_idx));
                param_idx += 1;
                format!("UPDATE \"{}\" SET {}", meta.table_name, set_sql)
            }
            None => format!("DELETE FROM \"{}\"", meta.table_name),
        };

        let mut raw_params = Vec::new();
        if let Some(where_clause) =
            self.where_body(&field_to_col, &mut raw_params, &mut param_idx, dialect)
        {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clause);
        }
        for p in raw_params {
            bind_values.push(BindValue::from(p));
        }

        db.record_query();
        let rows_affected = db.conn().execute(&sql, &bind_values).await?;
        crate::query_cache::invalidate_table(meta.table_name);
        Ok(rows_affected)
    }

    /// Performs a low-level INSERT query for `T` returning the generated primary key ID.
    pub async fn insert_raw<E: djangors_db::DbExecutor>(
        mut db: E,
        mut values: Vec<(&'static str, crate::expr::Value)>,
    ) -> Result<i64, OrmError> {
        let dialect = db.dialect();
        let meta = T::meta();
        for (field_name, value) in values.iter_mut() {
            if T::encrypted_fields().contains(field_name) {
                let plain = std::mem::replace(value, Value::Null);
                *value = crate::encryption::encrypt_value(plain)?;
            }
        }
        let pk_field = meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .expect("Primary key field not found");
        let pk_column = pk_field.column_name;

        let mut processed_values = Vec::new();
        for (field_name, val) in values {
            if let Some(f) = meta.fields.iter().find(|f| f.name == field_name) {
                if f.auto {
                    continue;
                }
                processed_values.push((f.column_name, val, field_name));
            } else if let Some(r) = meta.relations.iter().find(|r| r.field_name == field_name) {
                processed_values.push((r.field_name, val, field_name));
            } else {
                return Err(OrmError::FieldNotFound {
                    field: field_name.to_string(),
                    model: meta.struct_name,
                });
            }
        }

        let mut cols = Vec::new();
        let mut placeholders = Vec::new();
        let mut bind_values = Vec::new();
        for (i, (col_name, val, field_name)) in processed_values.into_iter().enumerate() {
            cols.push(format!("\"{}\"", col_name));
            placeholders.push(dialect.placeholder(i + 1));
            let nk = null_bind_kind_for(field_name, meta);
            let bv = match val {
                Value::Null => BindValue::Null(nk),
                other => BindValue::from(other),
            };
            bind_values.push(bv);
        }

        let sql = if cols.is_empty() {
            format!(
                "INSERT INTO \"{}\" DEFAULT VALUES RETURNING \"{}\"",
                meta.table_name, pk_column
            )
        } else {
            format!(
                "INSERT INTO \"{}\" ({}) VALUES ({}) RETURNING \"{}\"",
                meta.table_name,
                cols.join(", "),
                placeholders.join(", "),
                pk_column
            )
        };

        db.record_query();
        let row = db.conn().fetch_one(&sql, &bind_values).await?;
        let pk = row.try_i64(0)?.unwrap_or(0);
        crate::query_cache::invalidate_table(meta.table_name);
        Ok(pk)
    }

    /// Fetches the first row matching the current filter, or creates a new one from
    /// `defaults`.
    ///
    /// Returns `(T, true)` when created, `(T, false)` when the row already existed.
    /// The `defaults` closure receives no arguments and should return the values
    /// ready for `insert_raw` (a `Vec<(&'static str, Value)>`).
    pub async fn get_or_create<F>(
        &self,
        db: &djangors_db::Database,
        defaults: F,
    ) -> Result<(T, bool), OrmError>
    where
        F: FnOnce() -> Vec<(&'static str, super::expr::Value)> + Send,
        T: Send,
    {
        if let Some(existing) = self.first(db).await? {
            return Ok((existing, false));
        }
        let pk = QuerySet::<T>::insert_raw(db, defaults()).await?;
        let created = QuerySet::<T>::new()
            .filter(crate::q!(id = pk))
            .map_err(|e| OrmError::InvalidQuery(e.to_string()))?
            .get(db)
            .await?;
        Ok((created, true))
    }

    /// Like [`get_or_create`] but updates existing rows with `updates` instead of
    /// returning them unchanged.
    ///
    /// The `updates` closure receives no arguments and should return a
    /// `Vec<(&'static str, SetExpr)>` (the same form `set!` produces).
    pub async fn update_or_create<Fd, Fu>(
        &self,
        db: &djangors_db::Database,
        defaults: Fd,
        updates: Fu,
    ) -> Result<(T, bool), OrmError>
    where
        Fd: FnOnce() -> Vec<(&'static str, super::expr::Value)> + Send,
        Fu: FnOnce() -> Vec<(&'static str, super::expr::SetExpr)> + Send,
        T: Send,
    {
        if let Some(existing) = self.first(db).await? {
            self.update(db, updates()).await?;
            return Ok((existing, false));
        }
        let pk = QuerySet::<T>::insert_raw(db, defaults()).await?;
        let created = QuerySet::<T>::new()
            .filter(crate::q!(id = pk))
            .map_err(|e| OrmError::InvalidQuery(e.to_string()))?
            .get(db)
            .await?;
        Ok((created, true))
    }

    /// Inserts many rows in a single multi-row `INSERT` statement and returns
    /// their primary keys.
    pub async fn bulk_create<E: djangors_db::DbExecutor>(
        mut db: E,
        items: &[T],
    ) -> Result<Vec<i64>, OrmError>
    where
        T: Model,
    {
        if items.is_empty() {
            return Ok(Vec::new());
        }

        let dialect = db.dialect();
        let meta = T::meta();
        let pk_field = meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .expect("Primary key field not found");
        let pk_column = pk_field.column_name;

        let is_insertable = |field_name: &str| -> bool {
            match meta.fields.iter().find(|f| f.name == field_name) {
                Some(f) => !f.auto,
                None => meta.relations.iter().any(|r| r.field_name == field_name),
            }
        };

        let mut cols: Vec<String> = Vec::new();
        let mut col_order: Vec<&'static str> = Vec::new();
        for (field_name, _) in items[0].field_values() {
            if is_insertable(field_name) {
                let col_name = match meta.fields.iter().find(|f| f.name == field_name) {
                    Some(f) => f.column_name,
                    None => field_name,
                };
                cols.push(format!("\"{col_name}\""));
                col_order.push(field_name);
            }
        }

        let encrypted_names = T::encrypted_fields();
        let mut placeholder_groups = Vec::new();
        let mut bind_values = Vec::new();
        let mut param_idx = 1;
        for item in items {
            let values = item.field_values();
            let mut row_placeholders = Vec::new();
            for field_name in &col_order {
                let val = values
                    .iter()
                    .find(|(name, _)| name == field_name)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null);
                let val = if encrypted_names.contains(field_name) {
                    crate::encryption::encrypt_value(val)?
                } else {
                    val
                };
                row_placeholders.push(dialect.placeholder(param_idx));
                let nk = null_bind_kind_for(field_name, meta);
                let bv = match val {
                    Value::Null => BindValue::Null(nk),
                    other => BindValue::from(other),
                };
                bind_values.push(bv);
                param_idx += 1;
            }
            placeholder_groups.push(format!("({})", row_placeholders.join(", ")));
        }

        let sql = format!(
            "INSERT INTO \"{}\" ({}) VALUES {} RETURNING \"{}\"",
            meta.table_name,
            cols.join(", "),
            placeholder_groups.join(", "),
            pk_column
        );

        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;
        let mut pks = Vec::with_capacity(rows.len());
        for row in rows {
            pks.push(row.try_i64(0)?.unwrap_or(0));
        }
        crate::query_cache::invalidate_table(meta.table_name);
        Ok(pks)
    }

    /// Deletes a record from table `T` by primary key `pk`.
    ///
    /// On a `#[djangors(soft_delete)]` model this stamps `deleted_at` instead (a row
    /// that is already soft-deleted is left alone and counts as 0 rows). Use
    /// [`force_delete_by_pk`](Self::force_delete_by_pk) to remove the row.
    pub async fn delete_by_pk<E: djangors_db::DbExecutor>(db: E, pk: i64) -> Result<u64, OrmError> {
        Self::delete_by_pk_inner(db, pk, true).await
    }

    /// Permanently deletes a record by primary key, even on a soft-delete model.
    pub async fn force_delete_by_pk<E: djangors_db::DbExecutor>(
        db: E,
        pk: i64,
    ) -> Result<u64, OrmError> {
        Self::delete_by_pk_inner(db, pk, false).await
    }

    async fn delete_by_pk_inner<E: djangors_db::DbExecutor>(
        mut db: E,
        pk: i64,
        soft: bool,
    ) -> Result<u64, OrmError> {
        let dialect = db.dialect();
        let meta = T::meta();
        let pk_field = meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .expect("Primary key field not found");

        let soft_col = if soft { T::soft_delete_column() } else { None };
        let (sql, params) = match soft_col {
            Some(col) => (
                format!(
                    "UPDATE \"{}\" SET \"{}\" = {} WHERE \"{}\" = {} AND \"{}\" IS NULL",
                    meta.table_name,
                    col,
                    dialect.placeholder(1),
                    pk_field.column_name,
                    dialect.placeholder(2),
                    col
                ),
                vec![
                    BindValue::from(Value::DateTime(chrono::Utc::now())),
                    BindValue::I64(pk),
                ],
            ),
            None => (
                format!(
                    "DELETE FROM \"{}\" WHERE \"{}\" = {}",
                    meta.table_name,
                    pk_field.column_name,
                    dialect.placeholder(1)
                ),
                vec![BindValue::I64(pk)],
            ),
        };
        db.record_query();
        let rows_affected = db.conn().execute(&sql, &params).await?;
        crate::query_cache::invalidate_table(meta.table_name);
        Ok(rows_affected)
    }

    /// Eagerly loads the given relation, avoiding an N+1 query.
    pub async fn select_related<R: Model + FromRow + Clone, E: djangors_db::DbExecutor>(
        &self,
        mut db: E,
        relation_field: &'static str,
    ) -> Result<Vec<(T, Option<R>)>, OrmError> {
        self.check_for_update(&mut db)?;
        let dialect = db.dialect();
        let meta = T::meta();

        let relation = meta
            .relations
            .iter()
            .find(|r| r.field_name == relation_field)
            .ok_or_else(|| OrmError::FieldNotFound {
                field: relation_field.to_string(),
                model: meta.struct_name,
            })?;

        let target_meta = (relation.target)();
        let r_meta = R::meta();
        if target_meta.struct_name != r_meta.struct_name {
            return Err(OrmError::FieldNotFound {
                field: format!(
                    "{} (target model mismatch: expected {}, found {})",
                    relation_field, r_meta.struct_name, target_meta.struct_name
                ),
                model: meta.struct_name,
            });
        }

        let select_list = format!("*, \"{}\" AS __select_related_fk", relation_field);
        let (sql, params) = self.compile_select_with_order(&select_list, true, dialect);
        let bind_values: Vec<BindValue> = params.into_iter().map(BindValue::from).collect();

        db.record_query();
        let rows = db.conn().fetch_all(&sql, &bind_values).await?;

        let mut t_records = Vec::new();
        let mut relation_ids = std::collections::HashSet::new();

        for row in rows {
            let t = T::from_row(&row)?;
            let fk_id = row.try_i64_by_name("__select_related_fk")?;
            if let Some(id) = fk_id {
                relation_ids.insert(id);
            }
            t_records.push((t, fk_id));
        }

        if relation_ids.is_empty() {
            return Ok(t_records.into_iter().map(|(t, _)| (t, None)).collect());
        }

        let r_pk_field = r_meta
            .fields
            .iter()
            .find(|f| f.primary_key)
            .ok_or_else(|| OrmError::FieldNotFound {
                field: "primary_key".to_string(),
                model: r_meta.struct_name,
            })?;
        let r_pk_col = r_pk_field.column_name;

        let relation_ids_vec: Vec<i64> = relation_ids.into_iter().collect();
        let r_placeholders: Vec<String> = (1..=relation_ids_vec.len())
            .map(|i| dialect.placeholder(i))
            .collect();
        // A soft-deleted related row is treated as missing.
        let r_sql = format!(
            "SELECT * FROM \"{}\" WHERE \"{}\" IN ({}){}",
            r_meta.table_name,
            r_pk_col,
            r_placeholders.join(", "),
            live_rows_clause::<R>("")
        );
        let r_bind_values: Vec<BindValue> = relation_ids_vec
            .iter()
            .map(|&id| BindValue::I64(id))
            .collect();

        db.record_query();
        let r_rows = db.conn().fetch_all(&r_sql, &r_bind_values).await?;

        let mut r_map = std::collections::HashMap::new();
        for r_row in r_rows {
            let r = R::from_row(&r_row)?;
            let pk_val = r_row.try_i64_by_name(r_pk_col)?.unwrap_or(0);
            r_map.insert(pk_val, r);
        }

        let mut final_results = Vec::new();
        for (t, fk_id_opt) in t_records {
            let r_opt = match fk_id_opt {
                Some(id) => r_map.get(&id).cloned(),
                None => None,
            };
            final_results.push((t, r_opt));
        }

        Ok(final_results)
    }
}

/// ` AND "col" IS NULL` (or ` AND "table"."col" IS NULL` when `table` is non-empty) for a
/// soft-delete model, so related loaders skip deleted rows; empty for other models.
fn live_rows_clause<R: Model>(table: &str) -> String {
    match R::soft_delete_column() {
        Some(col) if table.is_empty() => format!(" AND \"{col}\" IS NULL"),
        Some(col) => format!(" AND \"{table}\".\"{col}\" IS NULL"),
        None => String::new(),
    }
}

/// Batch-loads the reverse side of a foreign-key relation for already-fetched parents.
pub async fn prefetch_related<T, R, E>(
    mut db: E,
    parents: &[T],
    related_name: &str,
) -> Result<std::collections::HashMap<i64, Vec<R>>, OrmError>
where
    T: Model,
    R: Model + FromRow + Clone,
    E: djangors_db::DbExecutor,
{
    let dialect = db.dialect();
    let parent_meta = T::meta();
    let child_meta = R::meta();
    let relation = child_meta
        .relations
        .iter()
        .find(|relation| {
            relation.related_name == Some(related_name)
                && (relation.target)().struct_name == parent_meta.struct_name
        })
        .ok_or_else(|| OrmError::FieldNotFound {
            field: format!("reverse relation '{}'", related_name),
            model: child_meta.struct_name,
        })?;
    let pk_field = parent_meta
        .fields
        .iter()
        .find(|field| field.primary_key)
        .ok_or_else(|| OrmError::FieldNotFound {
            field: "primary_key".to_string(),
            model: parent_meta.struct_name,
        })?;
    let parent_ids: Vec<i64> = parents
        .iter()
        .filter_map(|parent| {
            parent
                .field_values()
                .into_iter()
                .find(|(name, _)| *name == pk_field.name)
                .and_then(|(_, value)| match value {
                    Value::I64(id) => Some(id),
                    _ => None,
                })
        })
        .collect();

    if parent_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let placeholders: Vec<String> = (1..=parent_ids.len())
        .map(|i| dialect.placeholder(i))
        .collect();
    let sql = format!(
        "SELECT * FROM \"{}\" WHERE \"{}\" IN ({}){}",
        child_meta.table_name,
        relation.field_name,
        placeholders.join(", "),
        live_rows_clause::<R>("")
    );
    let bind_values: Vec<BindValue> = parent_ids.iter().map(|&id| BindValue::I64(id)).collect();

    db.record_query();
    let rows = db.conn().fetch_all(&sql, &bind_values).await?;
    let mut grouped = std::collections::HashMap::new();
    for row in rows {
        let child = R::from_row(&row)?;
        let parent_id = child
            .field_values()
            .into_iter()
            .find(|(name, _)| *name == relation.field_name)
            .and_then(|(_, value)| match value {
                Value::I64(id) => Some(id),
                _ => None,
            })
            .ok_or_else(|| OrmError::FieldNotFound {
                field: relation.field_name.to_string(),
                model: child_meta.struct_name,
            })?;
        grouped
            .entry(parent_id)
            .or_insert_with(Vec::new)
            .push(child);
    }
    Ok(grouped)
}

/// Fetches every `R` related to the `S` instance with `source_pk` through the named
/// many-to-many `field_name` on `S`. Django's `source.field.all()`.
pub async fn m2m_related<S: Model, R: Model + FromRow, E: djangors_db::DbExecutor>(
    mut db: E,
    source_pk: i64,
    field_name: &'static str,
) -> Result<Vec<R>, OrmError> {
    let dialect = db.dialect();
    let source_meta = S::meta();
    let relation = source_meta
        .relations
        .iter()
        .find(|r| r.field_name == field_name && r.kind == crate::meta::RelationKind::ManyToMany)
        .ok_or_else(|| OrmError::FieldNotFound {
            field: format!("M2M field '{}'", field_name),
            model: source_meta.struct_name,
        })?;
    let target_meta = (relation.target)();
    let r_meta = R::meta();
    if target_meta.struct_name != r_meta.struct_name {
        return Err(OrmError::FieldNotFound {
            field: format!(
                "{} (target model mismatch: expected {}, found {})",
                field_name, r_meta.struct_name, target_meta.struct_name
            ),
            model: source_meta.struct_name,
        });
    }

    let through = crate::meta::m2m_through_table(source_meta, relation);
    let source_col = format!("{}_id", source_meta.struct_name.to_lowercase());
    let target_col = format!("{}_id", target_meta.struct_name.to_lowercase());
    let r_pk = r_meta
        .fields
        .iter()
        .find(|f| f.primary_key)
        .ok_or_else(|| OrmError::FieldNotFound {
            field: "primary_key".to_string(),
            model: r_meta.struct_name,
        })?;

    let sql = format!(
        "SELECT \"{}\".* FROM \"{}\" \
         INNER JOIN \"{}\" ON \"{}\".\"{}\" = \"{}\".\"{}\" \
         WHERE \"{}\".\"{}\" = {}{}",
        r_meta.table_name,
        r_meta.table_name,
        through,
        through,
        target_col,
        r_meta.table_name,
        r_pk.column_name,
        through,
        source_col,
        dialect.placeholder(1),
        live_rows_clause::<R>(r_meta.table_name),
    );
    db.record_query();
    let rows = db
        .conn()
        .fetch_all(&sql, &[djangors_db::BindValue::I64(source_pk)])
        .await?;
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        results.push(R::from_row(&row)?);
    }
    Ok(results)
}

/// Adds a `(source_pk, related_pk)` pair to the named many-to-many join table.
///
/// Idempotent: inserting an existing pair is a no-op.
pub async fn m2m_add<S: Model, R: Model, E: djangors_db::DbExecutor>(
    mut db: E,
    source_pk: i64,
    field_name: &'static str,
    related_pk: i64,
) -> Result<(), OrmError> {
    let dialect = db.dialect();
    let source_meta = S::meta();
    let relation = source_meta
        .relations
        .iter()
        .find(|r| r.field_name == field_name && r.kind == crate::meta::RelationKind::ManyToMany)
        .ok_or_else(|| OrmError::FieldNotFound {
            field: format!("M2M field '{}'", field_name),
            model: source_meta.struct_name,
        })?;
    let target_meta = (relation.target)();
    let r_meta = R::meta();
    if target_meta.struct_name != r_meta.struct_name {
        return Err(OrmError::FieldNotFound {
            field: format!(
                "{} (target model mismatch: expected {}, found {})",
                field_name, r_meta.struct_name, target_meta.struct_name
            ),
            model: source_meta.struct_name,
        });
    }

    let through = crate::meta::m2m_through_table(source_meta, relation);
    let source_col = format!("{}_id", source_meta.struct_name.to_lowercase());
    let target_col = format!("{}_id", target_meta.struct_name.to_lowercase());
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql = format!(
        "INSERT INTO \"{through}\" ({source_col}, {target_col}) VALUES ({p1}, {p2}) ON CONFLICT DO NOTHING"
    );
    db.conn()
        .execute(
            &sql,
            &[
                djangors_db::BindValue::I64(source_pk),
                djangors_db::BindValue::I64(related_pk),
            ],
        )
        .await?;
    crate::query_cache::invalidate_table(&through);
    Ok(())
}

/// Removes the `(source_pk, related_pk)` pair from the named many-to-many join table.
pub async fn m2m_remove<S: Model, R: Model, E: djangors_db::DbExecutor>(
    mut db: E,
    source_pk: i64,
    field_name: &'static str,
    related_pk: i64,
) -> Result<(), OrmError> {
    let dialect = db.dialect();
    let source_meta = S::meta();
    let relation = source_meta
        .relations
        .iter()
        .find(|r| r.field_name == field_name && r.kind == crate::meta::RelationKind::ManyToMany)
        .ok_or_else(|| OrmError::FieldNotFound {
            field: format!("M2M field '{}'", field_name),
            model: source_meta.struct_name,
        })?;
    let target_meta = (relation.target)();
    let r_meta = R::meta();
    if target_meta.struct_name != r_meta.struct_name {
        return Err(OrmError::FieldNotFound {
            field: format!(
                "{} (target model mismatch: expected {}, found {})",
                field_name, r_meta.struct_name, target_meta.struct_name
            ),
            model: source_meta.struct_name,
        });
    }

    let through = crate::meta::m2m_through_table(source_meta, relation);
    let source_col = format!("{}_id", source_meta.struct_name.to_lowercase());
    let target_col = format!("{}_id", target_meta.struct_name.to_lowercase());
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql = format!(
        "DELETE FROM \"{through}\" WHERE \"{source_col}\" = {p1} AND \"{target_col}\" = {p2}"
    );
    db.conn()
        .execute(
            &sql,
            &[
                djangors_db::BindValue::I64(source_pk),
                djangors_db::BindValue::I64(related_pk),
            ],
        )
        .await?;
    crate::query_cache::invalidate_table(&through);
    Ok(())
}

/// One row of a grouped aggregate query produced by [`QuerySet::annotate`].
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRow {
    /// The group-by field names paired with this group's values.
    pub keys: Vec<(&'static str, Value)>,
    /// The aggregate aliases paired with their results.
    pub aggregates: Vec<(String, crate::aggregate::AggResult)>,
}

impl GroupRow {
    /// Looks up a group-key value by field name.
    pub fn key(&self, name: &str) -> Option<&Value> {
        self.keys.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
    }

    /// Looks up an aggregate result by alias.
    pub fn get(&self, alias: &str) -> Option<crate::aggregate::AggResult> {
        self.aggregates
            .iter()
            .find(|(k, _)| k == alias)
            .map(|(_, v)| *v)
    }
}

/// The model fields a scalar function reads.
fn func_fields(func: &crate::aggregate::FuncExpr) -> Vec<&'static str> {
    use crate::aggregate::FuncExpr;
    match func {
        FuncExpr::Coalesce { field, .. }
        | FuncExpr::Lower { field }
        | FuncExpr::Upper { field }
        | FuncExpr::Length { field }
        | FuncExpr::Cast { field, .. }
        | FuncExpr::Extract { field, .. }
        | FuncExpr::Trunc { field, .. }
        | FuncExpr::TrigramSimilarity { field, .. } => vec![field],
        FuncExpr::Concat { fields } => fields.clone(),
        FuncExpr::Window {
            func,
            partition_by,
            order_by,
            ..
        } => {
            let mut out = partition_by.clone();
            out.extend(order_by.iter().map(|(f, _)| *f));
            match func {
                crate::aggregate::WindowFunc::Lead { field, .. }
                | crate::aggregate::WindowFunc::Lag { field, .. }
                | crate::aggregate::WindowFunc::Sum(field)
                | crate::aggregate::WindowFunc::Avg(field)
                | crate::aggregate::WindowFunc::Count(field)
                    if *field != "*" =>
                {
                    out.push(field);
                }
                _ => {}
            }
            out
        }
        FuncExpr::Case { .. } | FuncExpr::Subquery { .. } | FuncExpr::RawSql { .. } => Vec::new(),
    }
}

fn agg_field(agg: &crate::aggregate::AggExpr) -> &'static str {
    match agg {
        crate::aggregate::AggExpr::Count { field }
        | crate::aggregate::AggExpr::Sum { field }
        | crate::aggregate::AggExpr::Avg { field }
        | crate::aggregate::AggExpr::Min { field }
        | crate::aggregate::AggExpr::Max { field } => field,
    }
}

fn agg_sql(
    agg: &crate::aggregate::AggExpr,
    field_to_col: &dyn Fn(&str) -> String,
    dialect: Dialect,
) -> String {
    match agg {
        crate::aggregate::AggExpr::Count { field } => {
            if *field == "*" {
                "COUNT(*)".to_string()
            } else {
                format!("COUNT({})", field_to_col(field))
            }
        }
        crate::aggregate::AggExpr::Sum { field } => {
            dialect.cast_float(&format!("SUM({})", field_to_col(field)))
        }
        crate::aggregate::AggExpr::Avg { field } => {
            dialect.cast_float(&format!("AVG({})", field_to_col(field)))
        }
        crate::aggregate::AggExpr::Min { field } => {
            dialect.cast_float(&format!("MIN({})", field_to_col(field)))
        }
        crate::aggregate::AggExpr::Max { field } => {
            dialect.cast_float(&format!("MAX({})", field_to_col(field)))
        }
    }
}

fn field_kind_of(name: &str, meta: &'static ModelMeta) -> Option<FieldKind> {
    meta.fields.iter().find(|f| f.name == name).map(|f| f.kind)
}

fn decode_value(row: &DbRow, idx: usize, kind: Option<FieldKind>) -> Value {
    match kind {
        Some(FieldKind::Char)
        | Some(FieldKind::Text)
        | Some(FieldKind::Email)
        | Some(FieldKind::Url)
        | Some(FieldKind::Slug)
        | Some(FieldKind::Ip)
        | Some(FieldKind::FileField) => row
            .try_string(idx)
            .ok()
            .flatten()
            .map(Value::Text)
            .unwrap_or(Value::Null),
        Some(FieldKind::Float) => row
            .try_f64(idx)
            .ok()
            .flatten()
            .map(Value::F64)
            .unwrap_or(Value::Null),
        Some(FieldKind::Boolean) => row
            .try_bool(idx)
            .ok()
            .flatten()
            .map(Value::Bool)
            .unwrap_or(Value::Null),
        Some(FieldKind::DateTime) => row
            .try_datetime(idx)
            .ok()
            .flatten()
            .map(Value::DateTime)
            .unwrap_or(Value::Null),
        _ => row
            .try_i64(idx)
            .ok()
            .flatten()
            .map(Value::I64)
            .unwrap_or(Value::Null),
    }
}

fn row_to_value(row: &djangors_db::DbRow, name: &str) -> Result<crate::expr::Value, OrmError> {
    if let Some(v) = row.try_i64_by_name(name)? {
        return Ok(crate::expr::Value::I64(v));
    }
    if let Some(v) = row.try_f64_by_name(name)? {
        return Ok(crate::expr::Value::F64(v));
    }
    if let Some(v) = row.try_string_by_name(name)? {
        return Ok(crate::expr::Value::Text(v));
    }
    if let Some(v) = row.try_bool_by_name(name)? {
        return Ok(crate::expr::Value::Bool(v));
    }
    if let Some(v) = row.try_datetime_by_name(name)? {
        return Ok(crate::expr::Value::DateTime(v));
    }
    Ok(crate::expr::Value::Null)
}

fn suffix_to_op(suffix: &str) -> CompareOp {
    match suffix {
        "eq" => CompareOp::Eq,
        "lt" => CompareOp::Lt,
        "lte" => CompareOp::Lte,
        "gt" => CompareOp::Gt,
        "gte" => CompareOp::Gte,
        "contains" => CompareOp::Contains,
        "icontains" => CompareOp::IContains,
        "startswith" => CompareOp::StartsWith,
        "endswith" => CompareOp::EndsWith,
        "istartswith" => CompareOp::IStartsWith,
        "iendswith" => CompareOp::IEndsWith,
        "ne" => CompareOp::Ne,
        "iexact" => CompareOp::IExact,
        "in" => CompareOp::In,
        "isnull" => CompareOp::IsNull,
        "regex" => CompareOp::Regex,
        "iregex" => CompareOp::IRegex,
        "range" => CompareOp::Range,
        "year" => CompareOp::Year,
        "month" => CompareOp::Month,
        "day" => CompareOp::Day,
        "week_day" => CompareOp::WeekDay,
        "date" => CompareOp::Date,
        "hour" => CompareOp::Hour,
        "has_key" => CompareOp::JsonHasKey,
        "json_contains" => CompareOp::JsonContains,
        _ => CompareOp::Eq,
    }
}

fn compile_expr_sql(
    expr: &Expr,
    outer_table: &str,
    subquery_table: Option<&str>,
    field_to_col: &dyn Fn(&str) -> String,
    params: &mut Vec<Value>,
    param_idx: &mut usize,
    dialect: Dialect,
) -> String {
    match expr {
        Expr::Compare { field, op, value } => {
            let col = field_to_col(field);

            match op {
                CompareOp::In => {
                    let items: Vec<Value> = match value {
                        Value::List(items) => items.clone(),
                        other => vec![other.clone()],
                    };
                    if items.is_empty() {
                        return "FALSE".to_string();
                    }
                    let mut placeholders = Vec::with_capacity(items.len());
                    for item in items {
                        params.push(item);
                        placeholders.push(dialect.placeholder(*param_idx));
                        *param_idx += 1;
                    }
                    return format!("{} IN ({})", col, placeholders.join(", "));
                }
                CompareOp::IsNull => {
                    let want_null = !matches!(value, Value::Bool(false));
                    return if want_null {
                        format!("{} IS NULL", col)
                    } else {
                        format!("{} IS NOT NULL", col)
                    };
                }
                CompareOp::Range => {
                    if let Value::List(items) = value {
                        if items.len() == 2 {
                            params.push(items[0].clone());
                            let p1 = dialect.placeholder(*param_idx);
                            *param_idx += 1;
                            params.push(items[1].clone());
                            let p2 = dialect.placeholder(*param_idx);
                            *param_idx += 1;
                            return format!("{col} BETWEEN {p1} AND {p2}");
                        }
                    }
                    return "FALSE".to_string();
                }
                CompareOp::Year
                | CompareOp::Month
                | CompareOp::Day
                | CompareOp::WeekDay
                | CompareOp::Hour => {
                    let part = match op {
                        CompareOp::Year => djangors_db::DatePart::Year,
                        CompareOp::Month => djangors_db::DatePart::Month,
                        CompareOp::Day => djangors_db::DatePart::Day,
                        CompareOp::WeekDay => djangors_db::DatePart::WeekDay,
                        CompareOp::Hour => djangors_db::DatePart::Hour,
                        _ => unreachable!(),
                    };
                    let extracted = dialect.extract_date_part(part, &col);
                    params.push(value.clone());
                    let p = dialect.placeholder(*param_idx);
                    *param_idx += 1;
                    return format!("{extracted} = {p}");
                }
                CompareOp::Date => {
                    params.push(value.clone());
                    let p = dialect.placeholder(*param_idx);
                    *param_idx += 1;
                    return match dialect {
                        Dialect::Postgres => format!("({col})::date = ({p})::date"),
                        Dialect::Sqlite | Dialect::MySql => format!("DATE({col}) = DATE({p})"),
                    };
                }
                CompareOp::JsonHasKey => {
                    let key = value.to_string();
                    return match dialect {
                        Dialect::Postgres => {
                            params.push(Value::Text(key));
                            let p = dialect.placeholder(*param_idx);
                            *param_idx += 1;
                            format!("{col} ? {p}")
                        }
                        Dialect::Sqlite => {
                            params.push(Value::Text(format!("$.{key}")));
                            let p = dialect.placeholder(*param_idx);
                            *param_idx += 1;
                            format!("json_extract({col}, {p}) IS NOT NULL")
                        }
                        Dialect::MySql => {
                            params.push(Value::Text(format!("$.{key}")));
                            let p = dialect.placeholder(*param_idx);
                            *param_idx += 1;
                            format!("JSON_CONTAINS_PATH({col}, 'one', {p})")
                        }
                    };
                }
                CompareOp::JsonContains => {
                    params.push(value.clone());
                    let p = dialect.placeholder(*param_idx);
                    *param_idx += 1;
                    return match dialect {
                        Dialect::Postgres => format!("{col} @> ({p})::jsonb"),
                        Dialect::MySql => format!("JSON_CONTAINS({col}, {p})"),
                        Dialect::Sqlite => format!("{col} LIKE '%' || {p} || '%'"),
                    };
                }
                _ => {}
            }

            let (op_sql, bind_val) = match op {
                CompareOp::In
                | CompareOp::IsNull
                | CompareOp::Range
                | CompareOp::Year
                | CompareOp::Month
                | CompareOp::Day
                | CompareOp::WeekDay
                | CompareOp::Date
                | CompareOp::Hour
                | CompareOp::JsonHasKey
                | CompareOp::JsonContains => unreachable!("handled above"),
                CompareOp::Ne => ("<>", value.clone()),
                CompareOp::Regex => ("~", value.clone()),
                CompareOp::IRegex => ("~*", value.clone()),
                CompareOp::IExact => (dialect.ilike(), value.clone()),
                CompareOp::Eq => ("=", value.clone()),
                CompareOp::Lt => ("<", value.clone()),
                CompareOp::Lte => ("<=", value.clone()),
                CompareOp::Gt => (">", value.clone()),
                CompareOp::Gte => (">=", value.clone()),
                CompareOp::Contains => {
                    let s = match value {
                        Value::Text(t) => format!("%{}%", t),
                        other => format!("%{}%", other),
                    };
                    ("LIKE", Value::Text(s))
                }
                CompareOp::IContains => {
                    let s = match value {
                        Value::Text(t) => format!("%{}%", t),
                        other => format!("%{}%", other),
                    };
                    (dialect.ilike(), Value::Text(s))
                }
                CompareOp::StartsWith => {
                    let s = match value {
                        Value::Text(t) => format!("{}%", t),
                        other => format!("{}%", other),
                    };
                    ("LIKE", Value::Text(s))
                }
                CompareOp::EndsWith => {
                    let s = match value {
                        Value::Text(t) => format!("%{}", t),
                        other => format!("%{}", other),
                    };
                    ("LIKE", Value::Text(s))
                }
                CompareOp::IStartsWith => {
                    let s = match value {
                        Value::Text(t) => format!("{}%", t),
                        other => format!("{}%", other),
                    };
                    (dialect.ilike(), Value::Text(s))
                }
                CompareOp::IEndsWith => {
                    let s = match value {
                        Value::Text(t) => format!("%{}", t),
                        other => format!("%{}", other),
                    };
                    (dialect.ilike(), Value::Text(s))
                }
            };
            params.push(bind_val);
            let placeholder = dialect.placeholder(*param_idx);
            *param_idx += 1;
            format!("{} {} {}", col, op_sql, placeholder)
        }
        Expr::And(exprs) => {
            if exprs.is_empty() {
                "TRUE".to_string()
            } else {
                let parts: Vec<String> = exprs
                    .iter()
                    .map(|e| {
                        format!(
                            "({})",
                            compile_expr_sql(
                                e,
                                outer_table,
                                subquery_table,
                                field_to_col,
                                params,
                                param_idx,
                                dialect
                            )
                        )
                    })
                    .collect();
                parts.join(" AND ")
            }
        }
        Expr::Or(exprs) => {
            if exprs.is_empty() {
                "FALSE".to_string()
            } else {
                let parts: Vec<String> = exprs
                    .iter()
                    .map(|e| {
                        format!(
                            "({})",
                            compile_expr_sql(
                                e,
                                outer_table,
                                subquery_table,
                                field_to_col,
                                params,
                                param_idx,
                                dialect
                            )
                        )
                    })
                    .collect();
                parts.join(" OR ")
            }
        }
        Expr::Not(inner) => {
            format!(
                "NOT ({})",
                compile_expr_sql(
                    inner,
                    outer_table,
                    subquery_table,
                    field_to_col,
                    params,
                    param_idx,
                    dialect
                )
            )
        }
        Expr::CompareField { left, op, right } => {
            let lhs = field_to_col(left);
            let rhs = field_to_col(right);
            let sql = match op {
                CompareOp::Eq => format!("{} = {}", lhs, rhs),
                CompareOp::Ne => format!("{} <> {}", lhs, rhs),
                CompareOp::Lt => format!("{} < {}", lhs, rhs),
                CompareOp::Lte => format!("{} <= {}", lhs, rhs),
                CompareOp::Gt => format!("{} > {}", lhs, rhs),
                CompareOp::Gte => format!("{} >= {}", lhs, rhs),
                CompareOp::Contains => format!("{} LIKE '%' || {} || '%'", lhs, rhs),
                CompareOp::IContains => {
                    format!("{} {} '%' || {} || '%'", lhs, dialect.ilike(), rhs)
                }
                CompareOp::StartsWith => format!("{} LIKE {} || '%'", lhs, rhs),
                CompareOp::EndsWith => format!("{} LIKE '%' || {}", lhs, rhs),
                CompareOp::IStartsWith => format!("{} {} {} || '%'", lhs, dialect.ilike(), rhs),
                CompareOp::IEndsWith => format!("{} {} '%' || {}", lhs, dialect.ilike(), rhs),
                CompareOp::IExact => format!("{} {} {}", lhs, dialect.ilike(), rhs),
                CompareOp::Regex => format!("{} ~ {}", lhs, rhs),
                CompareOp::IRegex => format!("{} ~* {}", lhs, rhs),
                _ => format!("{} = {}", lhs, rhs),
            };
            sql
        }
        Expr::OuterCompare {
            field,
            op,
            outer_field,
        } => {
            let sub_table = subquery_table.unwrap_or(outer_table);
            let lhs = format!("\"{}\".\"{}\"", sub_table, field);
            let rhs = format!("\"{}\".\"{}\"", outer_table, outer_field);
            let op_sql = match op {
                CompareOp::Eq => "=",
                CompareOp::Ne => "<>",
                CompareOp::Lt => "<",
                CompareOp::Lte => "<=",
                CompareOp::Gt => ">",
                CompareOp::Gte => ">=",
                CompareOp::Contains => return format!("{} LIKE '%' || {} || '%'", lhs, rhs),
                CompareOp::IContains => {
                    return format!("{} {} '%' || {} || '%'", lhs, dialect.ilike(), rhs)
                }
                CompareOp::StartsWith => return format!("{} LIKE {} || '%'", lhs, rhs),
                CompareOp::EndsWith => return format!("{} LIKE '%' || {}", lhs, rhs),
                CompareOp::IStartsWith => {
                    return format!("{} {} {} || '%'", lhs, dialect.ilike(), rhs)
                }
                CompareOp::IEndsWith => {
                    return format!("{} {} '%' || {}", lhs, dialect.ilike(), rhs)
                }
                CompareOp::IExact => return format!("{} {} {}", lhs, dialect.ilike(), rhs),
                CompareOp::Regex => return format!("{} ~ {}", lhs, rhs),
                CompareOp::IRegex => return format!("{} ~* {}", lhs, rhs),
                _ => "=",
            };
            format!("{} {} {}", lhs, op_sql, rhs)
        }
        Expr::Exists {
            negated,
            table: sub_table,
            filters,
        } => {
            // Note: EXISTS and NOT EXISTS with correlated subqueries use identical standard SQL
            // syntax (`EXISTS (SELECT 1 FROM "table" WHERE ...)`) in both PostgreSQL and SQLite,
            // so no dialect branch is required.
            let kw = if *negated { "NOT EXISTS" } else { "EXISTS" };
            let sub_field_to_col = |f: &str| format!("\"{}\".\"{}\"", sub_table, f);

            if filters.is_empty() {
                format!("{} (SELECT 1 FROM \"{}\")", kw, sub_table)
            } else {
                let combined = Expr::And(filters.clone());
                let where_clause = compile_expr_sql(
                    &combined,
                    outer_table,
                    Some(sub_table),
                    &sub_field_to_col,
                    params,
                    param_idx,
                    dialect,
                );
                format!(
                    "{} (SELECT 1 FROM \"{}\" WHERE {})",
                    kw, sub_table, where_clause
                )
            }
        }
    }
}

fn validate_sql_ident(name: &str) -> Result<(), OrmError> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(OrmError::InvalidQuery(format!(
            "invalid SQL identifier '{name}'"
        )));
    }
    Ok(())
}

/// Creates a SQLite FTS5 virtual table for full-text search across `columns`.
///
/// On non-SQLite backends this is a no-op (use [`QuerySet::search`] for Postgres `tsvector`).
pub async fn create_fts5_table(
    db: &djangors_db::Database,
    fts_table: &str,
    columns: &[&str],
) -> Result<(), OrmError> {
    validate_sql_ident(fts_table)?;
    for col in columns {
        validate_sql_ident(col)?;
    }
    let mut conn = db.conn();
    if conn.dialect() != Dialect::Sqlite {
        return Ok(());
    }
    let cols_sql = columns
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!("CREATE VIRTUAL TABLE IF NOT EXISTS \"{fts_table}\" USING fts5({cols_sql});");
    conn.execute(&sql, &[]).await?;
    Ok(())
}

/// Inserts or replaces a row in a SQLite FTS5 virtual table keyed by `rowid`.
pub async fn index_fts5_row(
    db: &djangors_db::Database,
    fts_table: &str,
    rowid: i64,
    columns: &[(&str, &str)],
) -> Result<(), OrmError> {
    validate_sql_ident(fts_table)?;
    for (col, _) in columns {
        validate_sql_ident(col)?;
    }
    let mut conn = db.conn();
    if conn.dialect() != Dialect::Sqlite {
        return Ok(());
    }
    let del_sql = format!("DELETE FROM \"{fts_table}\" WHERE rowid = ?;");
    conn.execute(&del_sql, &[BindValue::I64(rowid)]).await?;

    let mut col_names = vec!["\"rowid\"".to_string()];
    let mut placeholders = vec!["?".to_string()];
    let mut binds = vec![BindValue::I64(rowid)];
    for (col, val) in columns {
        col_names.push(format!("\"{col}\""));
        placeholders.push("?".to_string());
        binds.push(BindValue::Text((*val).to_string()));
    }
    let ins_sql = format!(
        "INSERT INTO \"{fts_table}\" ({}) VALUES ({});",
        col_names.join(", "),
        placeholders.join(", ")
    );
    conn.execute(&ins_sql, &binds).await?;
    Ok(())
}
