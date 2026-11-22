/// Aggregate function expression for QuerySet aggregations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggExpr {
    /// COUNT aggregate function.
    Count {
        /// Field name to count, or `"*"` for all rows.
        field: &'static str,
    },
    /// SUM aggregate function.
    Sum {
        /// Field name to sum.
        field: &'static str,
    },
    /// AVG aggregate function.
    Avg {
        /// Field name to average.
        field: &'static str,
    },
    /// MIN aggregate function.
    Min {
        /// Field name to find minimum value.
        field: &'static str,
    },
    /// MAX aggregate function.
    Max {
        /// Field name to find maximum value.
        field: &'static str,
    },
}

/// Result value returned by QuerySet aggregate execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggResult {
    /// Integer result value.
    I64(i64),
    /// Floating point result value.
    F64(f64),
    /// NULL aggregate result value.
    Null,
}

impl AggResult {
    /// Convert a row [`crate::expr::Value`] back into an aggregate result.
    pub fn from_value(v: &crate::expr::Value) -> Self {
        match v {
            crate::expr::Value::I64(n) => AggResult::I64(*n),
            crate::expr::Value::F64(f) => AggResult::F64(*f),
            crate::expr::Value::Null => AggResult::Null,
            other => {
                if let Ok(f) = other.to_string().parse::<f64>() {
                    AggResult::F64(f)
                } else {
                    AggResult::Null
                }
            }
        }
    }
}

/// Truncation granularity for [`FuncExpr::Trunc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncKind {
    /// Truncate to calendar year (`YYYY-01-01`).
    Year,
    /// Truncate to calendar month (`YYYY-MM-01`).
    Month,
    /// Truncate to calendar day (`YYYY-MM-DD`).
    Day,
    /// Truncate to hour (`YYYY-MM-DD HH:00:00`).
    Hour,
}

/// SQL window function kind for [`FuncExpr::Window`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowFunc {
    /// `ROW_NUMBER()`
    RowNumber,
    /// `RANK()`
    Rank,
    /// `DENSE_RANK()`
    DenseRank,
    /// `LEAD(field, offset)`
    Lead {
        /// Column/field name.
        field: &'static str,
        /// Row offset (defaults to 1).
        offset: usize,
    },
    /// `LAG(field, offset)`
    Lag {
        /// Column/field name.
        field: &'static str,
        /// Row offset (defaults to 1).
        offset: usize,
    },
    /// `SUM(field)` over the window.
    Sum(&'static str),
    /// `AVG(field)` over the window.
    Avg(&'static str),
    /// `COUNT(field)` over the window.
    Count(&'static str),
}

/// A single `WHEN <condition> THEN <value>` branch in a SQL `CASE` expression.
#[derive(Debug, Clone)]
pub struct WhenBranch {
    /// Raw SQL boolean condition (e.g. `"score" >= 90`).
    pub condition_sql: String,
    /// Result value when the condition evaluates to true.
    pub then: crate::expr::Value,
}

impl WhenBranch {
    /// Creates a `WHEN <condition_sql> THEN <then>` branch.
    pub fn new(condition_sql: impl Into<String>, then: impl Into<crate::expr::Value>) -> Self {
        Self {
            condition_sql: condition_sql.into(),
            then: then.into(),
        }
    }
}

/// Scalar database function expressions usable in `annotate_funcs` and `values`.
#[derive(Debug, Clone)]
pub enum FuncExpr {
    /// `COALESCE(field, default)` — returns the first non-null.
    Coalesce {
        /// Field name whose value is returned when non-null.
        field: &'static str,
        /// Value substituted when `field` is NULL.
        default: crate::expr::Value,
    },
    /// `LOWER(field)` — lowercase.
    Lower {
        /// Field name to lowercase.
        field: &'static str,
    },
    /// `UPPER(field)` — uppercase.
    Upper {
        /// Field name to uppercase.
        field: &'static str,
    },
    /// `field1 || field2 || ...` — string concatenation.
    Concat {
        /// Field names joined with the concatenation operator.
        fields: Vec<&'static str>,
    },
    /// `LENGTH(field)` — string length.
    Length {
        /// Field name whose character length is computed.
        field: &'static str,
    },
    /// `CAST(field AS sql_type)` — explicit type cast.
    Cast {
        /// Field name to cast.
        field: &'static str,
        /// Target SQL type (e.g. `"TEXT"`, `"INTEGER"`, `"DOUBLE PRECISION"`).
        sql_type: &'static str,
    },
    /// Extract an integer date/time component from a date/datetime field.
    Extract {
        /// Date/datetime field name.
        field: &'static str,
        /// Component to extract (`Year`, `Month`, `Day`, `Hour`, `WeekDay`).
        part: djangors_db::DatePart,
    },
    /// Truncate a timestamp/date to the given granularity (`DATE_TRUNC` / `strftime`).
    Trunc {
        /// Date/datetime field name.
        field: &'static str,
        /// Truncation unit.
        kind: TruncKind,
    },
    /// Conditional `CASE WHEN ... THEN ... ELSE ... END` expression.
    Case {
        /// Ordered `WHEN` branches.
        whens: Vec<WhenBranch>,
        /// Fallback `ELSE` value.
        default: crate::expr::Value,
        /// Column alias in the SELECT list.
        alias_name: String,
    },
    /// Window function `FUNC(...) OVER (PARTITION BY ... ORDER BY ...)`.
    Window {
        /// Window function to evaluate.
        func: WindowFunc,
        /// Fields to partition by.
        partition_by: Vec<&'static str>,
        /// `(field, descending)` ordering within each partition.
        order_by: Vec<(&'static str, bool)>,
        /// Column alias in the SELECT list.
        alias_name: String,
    },
    /// Correlated or uncorrelated scalar subquery `(SELECT ...)`.
    Subquery {
        /// Inner SELECT SQL statement (without outer parentheses).
        sql: String,
        /// Column alias in the SELECT list.
        alias_name: String,
    },
    /// PostgreSQL `pg_trgm` trigram similarity (`similarity(col, 'query')`).
    TrigramSimilarity {
        /// Text field name.
        field: &'static str,
        /// Search string literal.
        query: String,
    },
    /// Raw SQL expression (`RawSQL` equivalent).
    RawSql {
        /// Raw SQL expression fragment.
        sql: String,
        /// Column alias in the SELECT list.
        alias_name: String,
    },
}

fn format_sql_literal(v: &crate::expr::Value) -> String {
    match v {
        crate::expr::Value::I64(n) => n.to_string(),
        crate::expr::Value::F64(f) => f.to_string(),
        crate::expr::Value::Text(s) => format!("'{}'", s.replace('\'', "''")),
        crate::expr::Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).to_string(),
        crate::expr::Value::Null => "NULL".to_string(),
        _ => "NULL".to_string(),
    }
}

impl FuncExpr {
    /// Build a `COALESCE(field, default)` expression.
    pub fn coalesce(field: &'static str, default: impl Into<crate::expr::Value>) -> Self {
        Self::Coalesce {
            field,
            default: default.into(),
        }
    }

    /// Build a `LOWER(field)` expression.
    pub fn lower(field: &'static str) -> Self {
        Self::Lower { field }
    }

    /// Build an `UPPER(field)` expression.
    pub fn upper(field: &'static str) -> Self {
        Self::Upper { field }
    }

    /// Build a `fields[0] || fields[1] || ...` concatenation expression.
    pub fn concat(fields: &[&'static str]) -> Self {
        Self::Concat {
            fields: fields.to_vec(),
        }
    }

    /// Build a `LENGTH(field)` expression.
    pub fn length(field: &'static str) -> Self {
        Self::Length { field }
    }

    /// Build a `CAST(field AS sql_type)` expression.
    pub fn cast(field: &'static str, sql_type: &'static str) -> Self {
        Self::Cast { field, sql_type }
    }

    /// Build an `EXTRACT(part FROM field)` expression.
    pub fn extract(field: &'static str, part: djangors_db::DatePart) -> Self {
        Self::Extract { field, part }
    }

    /// Build a date truncation expression (`TruncDate`, `TruncMonth`, `TruncYear`, `TruncHour`).
    pub fn trunc(field: &'static str, kind: TruncKind) -> Self {
        Self::Trunc { field, kind }
    }

    /// Build a `CASE WHEN ... THEN ... ELSE ... END` expression.
    pub fn case(
        whens: Vec<WhenBranch>,
        default: impl Into<crate::expr::Value>,
        alias_name: impl Into<String>,
    ) -> Self {
        Self::Case {
            whens,
            default: default.into(),
            alias_name: alias_name.into(),
        }
    }

    /// Build a SQL window function expression (`OVER (PARTITION BY ... ORDER BY ...)`).
    pub fn window(
        func: WindowFunc,
        partition_by: &[&'static str],
        order_by: &[(&'static str, bool)],
        alias_name: impl Into<String>,
    ) -> Self {
        Self::Window {
            func,
            partition_by: partition_by.to_vec(),
            order_by: order_by.to_vec(),
            alias_name: alias_name.into(),
        }
    }

    /// Build a scalar `Subquery` expression (`(SELECT ...)`).
    pub fn subquery(sql: impl Into<String>, alias_name: impl Into<String>) -> Self {
        Self::Subquery {
            sql: sql.into(),
            alias_name: alias_name.into(),
        }
    }

    /// Build a PostgreSQL `pg_trgm` `similarity(field, query)` expression.
    pub fn trigram_similarity(field: &'static str, query: impl Into<String>) -> Self {
        Self::TrigramSimilarity {
            field,
            query: query.into(),
        }
    }

    /// Build a raw SQL expression (`RawSQL`).
    pub fn raw_sql(sql: impl Into<String>, alias_name: impl Into<String>) -> Self {
        Self::RawSql {
            sql: sql.into(),
            alias_name: alias_name.into(),
        }
    }

    /// Produce the SQL expression for this function call.
    pub fn to_sql(&self, dialect: &djangors_db::Dialect) -> String {
        match self {
            Self::Coalesce { field, default } => {
                let col = dialect.quote_ident(field);
                let def = format_sql_literal(default);
                format!("COALESCE({col}, {def})")
            }
            Self::Lower { field } => {
                format!("LOWER({})", dialect.quote_ident(field))
            }
            Self::Upper { field } => {
                format!("UPPER({})", dialect.quote_ident(field))
            }
            Self::Length { field } => {
                format!("LENGTH({})", dialect.quote_ident(field))
            }
            Self::Concat { fields } => {
                let quoted: Vec<String> = fields.iter().map(|f| dialect.quote_ident(f)).collect();
                match dialect {
                    djangors_db::Dialect::MySql => format!("CONCAT({})", quoted.join(", ")),
                    _ => quoted.join(" || "),
                }
            }
            Self::Cast { field, sql_type } => {
                format!("CAST({} AS {sql_type})", dialect.quote_ident(field))
            }
            Self::Extract { field, part } => {
                let col = dialect.quote_ident(field);
                dialect.extract_date_part(*part, &col)
            }
            Self::Trunc { field, kind } => {
                let col = dialect.quote_ident(field);
                match dialect {
                    djangors_db::Dialect::Postgres => {
                        let unit = match kind {
                            TruncKind::Year => "year",
                            TruncKind::Month => "month",
                            TruncKind::Day => "day",
                            TruncKind::Hour => "hour",
                        };
                        format!("DATE_TRUNC('{unit}', {col})")
                    }
                    djangors_db::Dialect::Sqlite => {
                        let fmt = match kind {
                            TruncKind::Year => "%Y-01-01",
                            TruncKind::Month => "%Y-%m-01",
                            TruncKind::Day => "%Y-%m-%d",
                            TruncKind::Hour => "%Y-%m-%d %H:00:00",
                        };
                        format!("strftime('{fmt}', {col})")
                    }
                    djangors_db::Dialect::MySql => {
                        let fmt = match kind {
                            TruncKind::Year => "%Y-01-01",
                            TruncKind::Month => "%Y-%m-01",
                            TruncKind::Day => "%Y-%m-%d",
                            TruncKind::Hour => "%Y-%m-%d %H:00:00",
                        };
                        format!("DATE_FORMAT({col}, '{fmt}')")
                    }
                }
            }
            Self::Case { whens, default, .. } => {
                let mut sql = String::from("CASE");
                for w in whens {
                    sql.push_str(&format!(
                        " WHEN {} THEN {}",
                        w.condition_sql,
                        format_sql_literal(&w.then)
                    ));
                }
                sql.push_str(&format!(" ELSE {} END", format_sql_literal(default)));
                sql
            }
            Self::Window {
                func,
                partition_by,
                order_by,
                ..
            } => {
                let fn_sql = match func {
                    WindowFunc::RowNumber => "ROW_NUMBER()".to_string(),
                    WindowFunc::Rank => "RANK()".to_string(),
                    WindowFunc::DenseRank => "DENSE_RANK()".to_string(),
                    WindowFunc::Lead { field, offset } => {
                        format!("LEAD({}, {offset})", dialect.quote_ident(field))
                    }
                    WindowFunc::Lag { field, offset } => {
                        format!("LAG({}, {offset})", dialect.quote_ident(field))
                    }
                    WindowFunc::Sum(field) => format!("SUM({})", dialect.quote_ident(field)),
                    WindowFunc::Avg(field) => format!("AVG({})", dialect.quote_ident(field)),
                    WindowFunc::Count(field) => {
                        if *field == "*" {
                            "COUNT(*)".to_string()
                        } else {
                            format!("COUNT({})", dialect.quote_ident(field))
                        }
                    }
                };
                let mut over_parts = Vec::new();
                if !partition_by.is_empty() {
                    let cols: Vec<String> = partition_by
                        .iter()
                        .map(|f| dialect.quote_ident(f))
                        .collect();
                    over_parts.push(format!("PARTITION BY {}", cols.join(", ")));
                }
                if !order_by.is_empty() {
                    let cols: Vec<String> = order_by
                        .iter()
                        .map(|(f, desc)| {
                            format!(
                                "{} {}",
                                dialect.quote_ident(f),
                                if *desc { "DESC" } else { "ASC" }
                            )
                        })
                        .collect();
                    over_parts.push(format!("ORDER BY {}", cols.join(", ")));
                }
                format!("{fn_sql} OVER ({})", over_parts.join(" "))
            }
            Self::Subquery { sql, .. } => format!("({sql})"),
            Self::TrigramSimilarity { field, query } => {
                let col = dialect.quote_ident(field);
                let escaped = query.replace('\'', "''");
                format!("similarity({col}, '{escaped}')")
            }
            Self::RawSql { sql, .. } => sql.clone(),
        }
    }

    /// A unique alias-safe name for this expression.
    pub fn alias(&self) -> String {
        match self {
            Self::Coalesce { field, .. } => format!("coalesce_{field}"),
            Self::Lower { field } => format!("lower_{field}"),
            Self::Upper { field } => format!("upper_{field}"),
            Self::Length { field } => format!("length_{field}"),
            Self::Concat { fields } => format!("concat_{}", fields.join("_")),
            Self::Cast { field, .. } => format!("cast_{field}"),
            Self::Extract { field, part } => format!("extract_{}_{:?}", field, part).to_lowercase(),
            Self::Trunc { field, kind } => format!("trunc_{}_{:?}", field, kind).to_lowercase(),
            Self::Case { alias_name, .. }
            | Self::Window { alias_name, .. }
            | Self::Subquery { alias_name, .. }
            | Self::RawSql { alias_name, .. } => alias_name.clone(),
            Self::TrigramSimilarity { field, .. } => format!("similarity_{field}"),
        }
    }
}
