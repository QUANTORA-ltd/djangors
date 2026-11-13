//! Database dialect representation for SQL syntax generation.

/// Supported SQL dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// PostgreSQL database dialect.
    Postgres,
    /// SQLite database dialect.
    Sqlite,
}

/// Part of a date/datetime to extract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePart {
    /// Year part.
    Year,
    /// Month part (1..=12).
    Month,
    /// Day part (1..=31).
    Day,
}

impl Dialect {
    /// Returns the parameter placeholder for this dialect.
    ///
    /// `$1`, `$2`, … for Postgres; `?` for SQLite.
    pub fn placeholder(&self, index: usize) -> String {
        match self {
            Dialect::Postgres => format!("${}", index),
            Dialect::Sqlite => "?".to_string(),
        }
    }

    /// Returns the case-insensitive LIKE operator for this dialect.
    ///
    /// `ILIKE` for Postgres; `LIKE` for SQLite (whose LIKE is already
    /// ASCII-case-insensitive, so this is the correct equivalent, not a downgrade).
    pub fn ilike(&self) -> &'static str {
        match self {
            Dialect::Postgres => "ILIKE",
            Dialect::Sqlite => "LIKE",
        }
    }

    /// Double-quotes an identifier name for both dialects.
    ///
    /// `"name"` for both dialects (SQLite accepts double-quoted identifiers).
    pub fn quote_ident(&self, name: &str) -> String {
        format!("\"{name}\"")
    }

    /// Casts an expression to a float/real type.
    ///
    /// `::float8` for Postgres; `CAST(… AS REAL)` shape for SQLite.
    pub fn cast_float(&self, expr: &str) -> String {
        match self {
            Dialect::Postgres => format!("{}::float8", expr),
            Dialect::Sqlite => format!("CAST({} AS REAL)", expr),
        }
    }

    /// SQL extracting an integer date part (`Year`, `Month`, `Day`) from `col`.
    pub fn extract_date_part(&self, part: DatePart, col: &str) -> String {
        match self {
            Dialect::Postgres => {
                let part_str = match part {
                    DatePart::Year => "YEAR",
                    DatePart::Month => "MONTH",
                    DatePart::Day => "DAY",
                };
                format!("EXTRACT({part_str} FROM {col})::int")
            }
            Dialect::Sqlite => {
                let fmt = match part {
                    DatePart::Year => "%Y",
                    DatePart::Month => "%m",
                    DatePart::Day => "%d",
                };
                format!("CAST(strftime('{fmt}', {col}) AS INTEGER)")
            }
        }
    }

    /// Returns the binary blob data type name for DDL.
    ///
    /// `BYTEA` for Postgres; `BLOB` for SQLite.
    pub fn bytea_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "BYTEA",
            Dialect::Sqlite => "BLOB",
        }
    }

    /// Returns the column type string for an auto-incrementing integer primary key.
    ///
    /// Postgres: `SERIAL PRIMARY KEY`; SQLite: `INTEGER PRIMARY KEY AUTOINCREMENT`.
    pub fn auto_pk_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "SERIAL PRIMARY KEY",
            Dialect::Sqlite => "INTEGER PRIMARY KEY AUTOINCREMENT",
        }
    }

    /// Returns the column type string for a 64-bit integer.
    ///
    /// Postgres: `BIGINT`; SQLite: `INTEGER` (always 64-bit storage).
    pub fn bigint_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "BIGINT",
            Dialect::Sqlite => "INTEGER",
        }
    }

    /// Returns the column type string for a boolean.
    ///
    /// `BOOLEAN` on both backends (SQLite stores it as 0/1 with numeric affinity).
    pub fn bool_type(&self) -> &'static str {
        match self {
            Dialect::Postgres | Dialect::Sqlite => "BOOLEAN",
        }
    }

    /// Returns the column type string for a timezone-aware timestamp.
    ///
    /// Postgres: `TIMESTAMPTZ`; SQLite: `TEXT`.
    pub fn timestamp_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "TIMESTAMPTZ",
            Dialect::Sqlite => "TEXT",
        }
    }

    /// Returns the SQL expression string for the current timestamp.
    ///
    /// Postgres: `now()`; SQLite: `CURRENT_TIMESTAMP`.
    pub fn current_timestamp(&self) -> &'static str {
        match self {
            Dialect::Postgres => "now()",
            Dialect::Sqlite => "CURRENT_TIMESTAMP",
        }
    }

    /// Infers the database dialect from a database URL string without connecting.
    pub fn from_url(url: &str) -> Dialect {
        let url = url.trim();
        if url.starts_with("sqlite://")
            || url.ends_with(".db")
            || url.ends_with(".sqlite")
            || url == ":memory:"
            || url == "sqlite::memory:"
        {
            Dialect::Sqlite
        } else {
            Dialect::Postgres
        }
    }
}
