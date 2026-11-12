use crate::error::OrmError;
use chrono::{DateTime, Utc};
use std::marker::PhantomData;
use std::ops::{BitAnd, BitOr, Not};

/// A dynamic value type used in ORM expressions and query parameters.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// 64-bit integer value.
    I64(i64),
    /// 64-bit floating point value.
    F64(f64),
    /// Text string value.
    Text(String),
    /// Boolean value.
    Bool(bool),
    /// UTC timestamp value.
    DateTime(DateTime<Utc>),
    /// Database NULL value.
    Null,
    /// A list of values, used as the right-hand side of an `__in` lookup.
    ///
    /// Never reaches the parameter binder: [`CompareOp::In`] expands the list
    /// into one placeholder per element while compiling the SQL, so the binder
    /// only ever sees the scalar elements.
    List(Vec<Value>),
}

impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(v: Vec<T>) -> Self {
        Value::List(v.into_iter().map(Into::into).collect())
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}

/// Smaller integer types widen to `i64`, which is what the database side binds
/// anyway. Without these, a model field declared `i32` (the common case for
/// Django's `IntegerField`) could not be used with `q!` without a manual cast.
macro_rules! impl_from_int {
    ($($t:ty),+ $(,)?) => {
        $(
            impl From<$t> for Value {
                fn from(v: $t) -> Self {
                    Value::I64(v as i64)
                }
            }
        )+
    };
}

impl_from_int!(i8, i16, i32, u8, u16, u32);

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::F64(v as f64)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<DateTime<Utc>> for Value {
    fn from(v: DateTime<Utc>) -> Self {
        Value::DateTime(v)
    }
}

impl From<Value> for djangors_db::BindValue {
    fn from(v: Value) -> Self {
        match v {
            Value::I64(v) => djangors_db::BindValue::I64(v),
            Value::F64(v) => djangors_db::BindValue::F64(v),
            Value::Text(v) => djangors_db::BindValue::Text(v),
            Value::Bool(v) => djangors_db::BindValue::Bool(v),
            Value::DateTime(v) => djangors_db::BindValue::DateTime(v),
            Value::Null => djangors_db::BindValue::Null(djangors_db::NullKind::I64),
            // Value::List must never reach here — it is expanded into scalars by compile_expr_sql —
            // so the conversion maps it to Null(NullKind::I64) with a comment, exactly as the current bind sites do.
            Value::List(_) => djangors_db::BindValue::Null(djangors_db::NullKind::I64),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::I64(v) => write!(f, "{}", v),
            Value::F64(v) => write!(f, "{}", v),
            Value::Text(v) => write!(f, "{}", v),
            Value::Bool(v) => write!(f, "{}", v),
            Value::DateTime(v) => write!(f, "{}", v.format("%Y-%m-%d %H:%M:%S")),
            Value::Null => write!(f, "-"),
            Value::List(items) => {
                let rendered: Vec<String> = items.iter().map(|i| i.to_string()).collect();
                write!(f, "{}", rendered.join(", "))
            }
        }
    }
}

/// Comparison operators for query filtering expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    /// Exact equality comparison (`=`).
    Eq,
    /// Less-than comparison (`<`).
    Lt,
    /// Less-than-or-equal comparison (`<=`).
    Lte,
    /// Greater-than comparison (`>`).
    Gt,
    /// Greater-than-or-equal comparison (`>=`).
    Gte,
    /// Case-sensitive substring search (`LIKE '%val%'`).
    Contains,
    /// Case-insensitive substring search (`ILIKE '%val%'`).
    IContains,
    /// Prefix string match (`LIKE 'val%'`).
    StartsWith,
    /// Suffix string match (`LIKE '%val'`).
    EndsWith,
    /// Inequality comparison (`<>`). Lookup suffix `ne`.
    Ne,
    /// Case-insensitive exact match (`ILIKE 'val'`, no wildcards). Lookup suffix `iexact`.
    IExact,
    /// Membership test (`IN (...)`). Lookup suffix `in`; expects [`Value::List`].
    ///
    /// An empty list compiles to `FALSE` rather than the syntactically invalid
    /// `IN ()`, matching Django's behaviour for `__in=[]`.
    In,
    /// NULL test (`IS NULL` / `IS NOT NULL`). Lookup suffix `isnull`; expects
    /// [`Value::Bool`], where `false` inverts the test.
    IsNull,
    /// Case-sensitive POSIX regular-expression match (`~`). Lookup suffix `regex`.
    Regex,
    /// Case-insensitive POSIX regular-expression match (`~*`). Lookup suffix `iregex`.
    IRegex,
}

/// Resolved boolean expression tree for query WHERE clauses.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A single field comparison expression.
    Compare {
        /// Target field name on the model.
        field: &'static str,
        /// Comparison operator.
        op: CompareOp,
        /// Comparison value.
        value: Value,
    },
    /// Conjunction of multiple expressions (AND).
    And(Vec<Expr>),
    /// Disjunction of multiple expressions (OR).
    Or(Vec<Expr>),
    /// Negation of an expression (NOT).
    Not(Box<Expr>),
    /// Comparison of one column against another on the same row.
    ///
    /// This is Django's `F()` on the *filter* side (`.filter(q_f!(paid_at gte
    /// due_at))`), as opposed to [`SetExpr::FieldOp`], which is `F()` on the
    /// *update* side. Both operands are resolved field names, so neither
    /// contributes a bind parameter.
    CompareField {
        /// Left-hand field name on the model.
        left: &'static str,
        /// Comparison operator.
        op: CompareOp,
        /// Right-hand field name on the model.
        right: &'static str,
    },
    /// Correlated subquery existence test (`EXISTS` / `NOT EXISTS`).
    Exists {
        /// If `true`, compiles to `NOT EXISTS`; otherwise `EXISTS`.
        negated: bool,
        /// Subquery table name from `S::meta().table_name`.
        table: &'static str,
        /// The subquery's internal WHERE filter expressions, ANDed.
        filters: Vec<Expr>,
    },
    /// Comparison of a column on the subquery table against a column on the outer table.
    OuterCompare {
        /// Column name on the SUBQUERY table.
        field: &'static str,
        /// Comparison operator.
        op: CompareOp,
        /// Column name on the OUTER table.
        outer_field: &'static str,
    },
}

impl BitAnd for Expr {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Expr::And(mut l), Expr::And(r)) => {
                l.extend(r);
                Expr::And(l)
            }
            (Expr::And(mut l), r) => {
                l.push(r);
                Expr::And(l)
            }
            (l, Expr::And(mut r)) => {
                r.insert(0, l);
                Expr::And(r)
            }
            (l, r) => Expr::And(vec![l, r]),
        }
    }
}

impl BitOr for Expr {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Expr::Or(mut l), Expr::Or(r)) => {
                l.extend(r);
                Expr::Or(l)
            }
            (Expr::Or(mut l), r) => {
                l.push(r);
                Expr::Or(l)
            }
            (l, Expr::Or(mut r)) => {
                r.insert(0, l);
                Expr::Or(r)
            }
            (l, r) => Expr::Or(vec![l, r]),
        }
    }
}

impl Not for Expr {
    type Output = Self;

    fn not(self) -> Self {
        Expr::Not(Box::new(self))
    }
}

/// A reference to a field on the outer query's model in a correlated subquery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OuterRef(pub &'static str);

/// Raw field comparison before lookup suffix resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedCompare {
    /// Field name (possibly containing lookup suffix like `field__contains`).
    pub field: &'static str,
    /// Comparison value.
    pub value: Value,
}

/// Raw column-to-column comparison before lookup suffix resolution.
///
/// Produced by the [`q_f!`](crate::q_f) macro; the counterpart to
/// [`UnresolvedCompare`] for the right-hand side being a field rather than a
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedFieldCompare {
    /// Left-hand field name (possibly carrying a lookup suffix).
    pub left: &'static str,
    /// Right-hand field name (never carries a suffix).
    pub right: &'static str,
}

/// Raw comparison between a subquery field and an outer field before lookup suffix resolution.
///
/// Produced by the [`q_outer!`](crate::q_outer) macro; the counterpart to
/// [`UnresolvedCompare`] for correlated subqueries referencing the outer table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedOuterCompare {
    /// Field name on the subquery model (possibly carrying a lookup suffix like `author_id__eq`).
    pub field: &'static str,
    /// Field name on the outer query model.
    pub outer_field: &'static str,
}

/// Unresolved filter expression generated by the `q!` macro.
///
/// This is a tree: [`And`](Self::And) is the leaf group produced by a single
/// `q!(...)` invocation, while [`All`](Self::All), [`Any`](Self::Any) and
/// [`Negate`](Self::Negate) compose those leaves. The [`BitAnd`], [`BitOr`] and
/// [`Not`] operators build the composite variants, which is what makes Django's
/// `Q(a=1) | Q(b=2)` spell as `q!(a = 1) | q!(b = 2)`.
///
/// ```
/// # use djangors_orm::q;
/// let expr = (q!(status = "open") | q!(status = "pending")) & !q!(archived = true);
/// # let _ = expr;
/// ```
#[derive(Debug, Clone)]
pub enum UnresolvedExpr {
    /// Conjunction of unresolved field comparisons (the `q!` leaf).
    And(Vec<UnresolvedCompare>),
    /// Conjunction of unresolved column-to-column comparisons (the `q_f!` leaf).
    AndFields(Vec<UnresolvedFieldCompare>),
    /// Conjunction of outer-field comparisons (the `q_outer!` leaf).
    AndOuter(Vec<UnresolvedOuterCompare>),
    /// Conjunction of sub-expressions.
    All(Vec<UnresolvedExpr>),
    /// Disjunction of sub-expressions.
    Any(Vec<UnresolvedExpr>),
    /// Negation of a sub-expression.
    Negate(Box<UnresolvedExpr>),
    /// Correlated subquery expression generated by [`Exists`] or [`NotExists`].
    Exists {
        /// If `true`, compiles to `NOT EXISTS`; otherwise `EXISTS`.
        negated: bool,
        /// Subquery model metadata getter function.
        subquery_meta: fn() -> &'static crate::meta::ModelMeta,
        /// Soft-delete column of the subquery model, if it is a soft-delete model.
        /// Rows where this column is set are excluded from the subquery.
        soft_delete_col: Option<&'static str>,
        /// Subquery's unresolved filter expressions.
        filters: Vec<UnresolvedExpr>,
    },
}

impl PartialEq for UnresolvedExpr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::And(l), Self::And(r)) => l == r,
            (Self::AndFields(l), Self::AndFields(r)) => l == r,
            (Self::AndOuter(l), Self::AndOuter(r)) => l == r,
            (Self::All(l), Self::All(r)) => l == r,
            (Self::Any(l), Self::Any(r)) => l == r,
            (Self::Negate(l), Self::Negate(r)) => l == r,
            (
                Self::Exists {
                    negated: l_neg,
                    subquery_meta: l_meta,
                    soft_delete_col: l_sd,
                    filters: l_f,
                },
                Self::Exists {
                    negated: r_neg,
                    subquery_meta: r_meta,
                    soft_delete_col: r_sd,
                    filters: r_f,
                },
            ) => {
                l_neg == r_neg
                    && std::ptr::eq(*l_meta as *const (), *r_meta as *const ())
                    && l_sd == r_sd
                    && l_f == r_f
            }
            _ => false,
        }
    }
}

impl BitAnd for UnresolvedExpr {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        match (self, rhs) {
            (UnresolvedExpr::All(mut l), UnresolvedExpr::All(r)) => {
                l.extend(r);
                UnresolvedExpr::All(l)
            }
            (UnresolvedExpr::All(mut l), r) => {
                l.push(r);
                UnresolvedExpr::All(l)
            }
            (l, UnresolvedExpr::All(mut r)) => {
                r.insert(0, l);
                UnresolvedExpr::All(r)
            }
            (l, r) => UnresolvedExpr::All(vec![l, r]),
        }
    }
}

impl BitOr for UnresolvedExpr {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (UnresolvedExpr::Any(mut l), UnresolvedExpr::Any(r)) => {
                l.extend(r);
                UnresolvedExpr::Any(l)
            }
            (UnresolvedExpr::Any(mut l), r) => {
                l.push(r);
                UnresolvedExpr::Any(l)
            }
            (l, UnresolvedExpr::Any(mut r)) => {
                r.insert(0, l);
                UnresolvedExpr::Any(r)
            }
            (l, r) => UnresolvedExpr::Any(vec![l, r]),
        }
    }
}

impl Not for UnresolvedExpr {
    type Output = Self;

    fn not(self) -> Self {
        UnresolvedExpr::Negate(Box::new(self))
    }
}

/// Represents an `EXISTS` subquery expression over subquery model `S`.
///
/// # Note
/// An `Exists` subquery whose filters contain no [`OuterRef`] (via [`q_outer!`])
/// is an uncorrelated subquery. While legal SQL, an uncorrelated subquery evaluates
/// once for the entire query and does not correlate rows between the subquery and outer query.
#[derive(Debug, Clone)]
pub struct Exists<S: crate::meta::Model> {
    pub(crate) negated: bool,
    pub(crate) filters: Vec<UnresolvedExpr>,
    pub(crate) _marker: PhantomData<S>,
}

impl<S: crate::meta::Model> Exists<S> {
    /// Creates a new `Exists` subquery builder for subquery model `S`.
    pub fn new() -> Self {
        Self {
            negated: false,
            filters: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// Adds a filter expression to the subquery.
    pub fn filter(mut self, expr: impl Into<UnresolvedExpr>) -> Result<Self, OrmError> {
        let uexpr = expr.into();
        Self::validate_subquery_expr(&uexpr)?;
        self.filters.push(uexpr);
        Ok(self)
    }

    fn validate_subquery_expr(expr: &UnresolvedExpr) -> Result<(), OrmError> {
        let meta = S::meta();
        validate_subquery_expr_meta(meta, expr)
    }
}

impl<S: crate::meta::Model> Default for Exists<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: crate::meta::Model> From<Exists<S>> for UnresolvedExpr {
    fn from(e: Exists<S>) -> Self {
        UnresolvedExpr::Exists {
            negated: e.negated,
            subquery_meta: S::meta,
            soft_delete_col: S::soft_delete_column(),
            filters: e.filters,
        }
    }
}

/// Represents a `NOT EXISTS` subquery expression over subquery model `S`.
///
/// # Note
/// A `NotExists` subquery whose filters contain no [`OuterRef`] (via [`q_outer!`])
/// is an uncorrelated subquery. While legal SQL, an uncorrelated subquery evaluates
/// once for the entire query and does not correlate rows between the subquery and outer query.
#[derive(Debug, Clone)]
pub struct NotExists<S: crate::meta::Model> {
    inner: Exists<S>,
}

impl<S: crate::meta::Model> NotExists<S> {
    /// Creates a new `NotExists` subquery builder for subquery model `S`.
    pub fn new() -> Self {
        Self {
            inner: Exists {
                negated: true,
                filters: Vec::new(),
                _marker: PhantomData,
            },
        }
    }

    /// Adds a filter expression to the subquery.
    pub fn filter(mut self, expr: impl Into<UnresolvedExpr>) -> Result<Self, OrmError> {
        self.inner = self.inner.filter(expr)?;
        Ok(self)
    }
}

impl<S: crate::meta::Model> Default for NotExists<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: crate::meta::Model> From<NotExists<S>> for UnresolvedExpr {
    fn from(ne: NotExists<S>) -> Self {
        ne.inner.into()
    }
}

fn validate_subquery_expr_meta(
    meta: &'static crate::meta::ModelMeta,
    expr: &UnresolvedExpr,
) -> Result<(), OrmError> {
    match expr {
        UnresolvedExpr::And(compares) => {
            for comp in compares {
                let (field_name, _) = split_field_lookup(comp.field);
                check_field_exists_on_meta(meta, field_name)?;
            }
        }
        UnresolvedExpr::AndFields(compares) => {
            for comp in compares {
                let (left_name, _) = split_field_lookup(comp.left);
                check_field_exists_on_meta(meta, left_name)?;
                check_field_exists_on_meta(meta, comp.right)?;
            }
        }
        UnresolvedExpr::AndOuter(compares) => {
            for comp in compares {
                let (field_name, _) = split_field_lookup(comp.field);
                check_field_exists_on_meta(meta, field_name)?;
            }
        }
        UnresolvedExpr::All(nodes) | UnresolvedExpr::Any(nodes) => {
            for node in nodes {
                validate_subquery_expr_meta(meta, node)?;
            }
        }
        UnresolvedExpr::Negate(inner) => {
            validate_subquery_expr_meta(meta, inner)?;
        }
        UnresolvedExpr::Exists {
            filters,
            subquery_meta,
            ..
        } => {
            let sub_meta = subquery_meta();
            for f in filters {
                validate_subquery_expr_meta(sub_meta, f)?;
            }
        }
    }
    Ok(())
}

fn check_field_exists_on_meta(
    meta: &'static crate::meta::ModelMeta,
    field: &str,
) -> Result<(), OrmError> {
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

/// Splits a field lookup string (e.g. `"age__gte"`) into field name `"age"` and suffix `"gte"`.
pub fn split_field_lookup(s: &'static str) -> (&'static str, &'static str) {
    if let Some(idx) = s.rfind("__") {
        let field = &s[..idx];
        let suffix = &s[idx + 2..];
        match suffix {
            "eq" | "lt" | "lte" | "gt" | "gte" | "contains" | "icontains" | "startswith"
            | "endswith" | "ne" | "iexact" | "in" | "isnull" | "regex" | "iregex" => {
                (field, suffix)
            }
            _ => (s, "eq"),
        }
    } else {
        (s, "eq")
    }
}

/// Constructs an [`UnresolvedExpr`] comparing a subquery field to an [`OuterRef`].
///
/// ```rust,illustrative
/// # use djangors_orm::{q_outer, OuterRef};
/// let correlation = q_outer!(author_id = OuterRef("id"));
/// # let _ = correlation;
/// ```
#[macro_export]
macro_rules! q_outer {
    ($($field:ident = OuterRef($outer:expr)),+ $(,)?) => {
        $crate::expr::UnresolvedExpr::AndOuter(vec![
            $(
                $crate::expr::UnresolvedOuterCompare {
                    field: stringify!($field),
                    outer_field: $outer,
                }
            ),+
        ])
    };
    ($($field:ident = $outer:expr),+ $(,)?) => {
        $crate::expr::UnresolvedExpr::AndOuter(vec![
            $(
                $crate::expr::UnresolvedOuterCompare {
                    field: stringify!($field),
                    outer_field: $outer.0,
                }
            ),+
        ])
    };
}

/// Constructs an [`UnresolvedExpr`] comparing one column to another on the same row.
///
/// The filter-side counterpart to [`F`]: where `q!(count = 5)` compares a column
/// to a bound value, `q_f!(paid_at__gte due_at)` compares two columns. As in
/// Django, the lookup suffix rides on the left-hand field.
///
/// ```
/// # use djangors_orm::q_f;
/// let overdue = q_f!(paid_at__gte due_at);
/// # let _ = overdue;
/// ```
#[macro_export]
macro_rules! q_f {
    ($($left:ident $right:ident),+ $(,)?) => {
        $crate::expr::UnresolvedExpr::AndFields(vec![
            $(
                $crate::expr::UnresolvedFieldCompare {
                    left: stringify!($left),
                    right: stringify!($right),
                }
            ),+
        ])
    };
}

/// Constructs an [`UnresolvedExpr`] for filtering querysets.
#[macro_export]
macro_rules! q {
    ($($field:ident = $value:expr),+ $(,)?) => {
        $crate::expr::UnresolvedExpr::And(vec![
            $(
                $crate::expr::UnresolvedCompare {
                    field: stringify!($field),
                    value: $crate::expr::Value::from($value),
                }
            ),+
        ])
    };
}

/// Arithmetic operators for UPDATE set expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    /// Addition operator (`+`).
    Add,
    /// Subtraction operator (`-`).
    Sub,
    /// Multiplication operator (`*`).
    Mul,
    /// Division operator (`/`).
    Div,
}

/// Expression specifying a value update in an UPDATE query.
#[derive(Debug, Clone, PartialEq)]
pub enum SetExpr {
    /// A literal new value.
    Literal(Value),
    /// An in-database field arithmetic operation (e.g. `col = col + 1`).
    FieldOp {
        /// Target field name on the model.
        field: &'static str,
        /// Arithmetic operator.
        op: ArithOp,
        /// Right-hand operand value.
        operand: Value,
    },
}

/// Django's F() — a reference to a field's CURRENT value in the database.
pub struct F(pub &'static str);

impl std::ops::Add<i64> for F {
    type Output = SetExpr;
    fn add(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Add,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Sub<i64> for F {
    type Output = SetExpr;
    fn sub(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Sub,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Mul<i64> for F {
    type Output = SetExpr;
    fn mul(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Mul,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Div<i64> for F {
    type Output = SetExpr;
    fn div(self, rhs: i64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Div,
            operand: Value::I64(rhs),
        }
    }
}

impl std::ops::Add<f64> for F {
    type Output = SetExpr;
    fn add(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Add,
            operand: Value::F64(rhs),
        }
    }
}

impl std::ops::Sub<f64> for F {
    type Output = SetExpr;
    fn sub(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Sub,
            operand: Value::F64(rhs),
        }
    }
}

impl std::ops::Mul<f64> for F {
    type Output = SetExpr;
    fn mul(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Mul,
            operand: Value::F64(rhs),
        }
    }
}

impl std::ops::Div<f64> for F {
    type Output = SetExpr;
    fn div(self, rhs: f64) -> SetExpr {
        SetExpr::FieldOp {
            field: self.0,
            op: ArithOp::Div,
            operand: Value::F64(rhs),
        }
    }
}

/// Trait implemented by types that can be converted into a [`SetExpr`].
pub trait IntoSetExpr {
    /// Converts `self` into a [`SetExpr`].
    fn into_set_expr(self) -> SetExpr;
}

impl IntoSetExpr for SetExpr {
    fn into_set_expr(self) -> SetExpr {
        self
    }
}

impl<T: Into<Value>> IntoSetExpr for T {
    fn into_set_expr(self) -> SetExpr {
        SetExpr::Literal(self.into())
    }
}

/// Constructs a vector of field assignment tuples for [`QuerySet::update`](crate::QuerySet::update).
#[macro_export]
macro_rules! set {
    ($($field:ident = $value:expr),+ $(,)?) => {
        vec![
            $(
                (stringify!($field), $crate::expr::IntoSetExpr::into_set_expr($value))
            ),+
        ]
    };
}
