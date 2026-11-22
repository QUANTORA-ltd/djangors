//! Database dialect representation for SQL syntax generation.

/// Supported SQL dialects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// PostgreSQL database dialect.
    Postgres,
    /// SQLite database dialect.
    Sqlite,
    /// MySQL / MariaDB database dialect.
    MySql,
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
    /// Hour part (0..=23).
    Hour,
    /// Day of week (Django convention: 1 = Sunday .. 7 = Saturday).
    WeekDay,
}

impl Dialect {
    /// Returns the parameter placeholder for this dialect.
    ///
    /// `$1`, `$2`, … for Postgres; `?` for SQLite and MySQL.
    pub fn placeholder(&self, index: usize) -> String {
        match self {
            Dialect::Postgres => format!("${}", index),
            Dialect::Sqlite | Dialect::MySql => "?".to_string(),
        }
    }

    /// Returns the case-insensitive LIKE operator for this dialect.
    ///
    /// `ILIKE` for Postgres; `LIKE` for SQLite and MySQL (whose default collations
    /// are case-insensitive for ASCII).
    pub fn ilike(&self) -> &'static str {
        match self {
            Dialect::Postgres => "ILIKE",
            Dialect::Sqlite | Dialect::MySql => "LIKE",
        }
    }

    /// Quotes an identifier name appropriately for this dialect.
    ///
    /// `"name"` for Postgres and SQLite; `` `name` `` for MySQL.
    pub fn quote_ident(&self, name: &str) -> String {
        match self {
            Dialect::Postgres | Dialect::Sqlite => format!("\"{name}\""),
            Dialect::MySql => format!("`{name}`"),
        }
    }

    /// Casts an expression to a float/real type.
    ///
    /// `::float8` for Postgres; `CAST(… AS REAL)` for SQLite; `CAST(… AS DOUBLE)` for MySQL.
    pub fn cast_float(&self, expr: &str) -> String {
        match self {
            Dialect::Postgres => format!("{}::float8", expr),
            Dialect::Sqlite => format!("CAST({} AS REAL)", expr),
            Dialect::MySql => format!("CAST({} AS DOUBLE)", expr),
        }
    }

    /// SQL extracting an integer date part from `col`.
    pub fn extract_date_part(&self, part: DatePart, col: &str) -> String {
        match self {
            Dialect::Postgres => match part {
                DatePart::Year => format!("EXTRACT(YEAR FROM {col})::int"),
                DatePart::Month => format!("EXTRACT(MONTH FROM {col})::int"),
                DatePart::Day => format!("EXTRACT(DAY FROM {col})::int"),
                DatePart::Hour => format!("EXTRACT(HOUR FROM {col})::int"),
                // Postgres DOW is 0=Sunday..6=Saturday; Django week_day is 1=Sunday..7=Saturday.
                DatePart::WeekDay => format!("(EXTRACT(DOW FROM {col})::int + 1)"),
            },
            Dialect::Sqlite => match part {
                DatePart::Year => format!("CAST(strftime('%Y', {col}) AS INTEGER)"),
                DatePart::Month => format!("CAST(strftime('%m', {col}) AS INTEGER)"),
                DatePart::Day => format!("CAST(strftime('%d', {col}) AS INTEGER)"),
                DatePart::Hour => format!("CAST(strftime('%H', {col}) AS INTEGER)"),
                // SQLite %w is 0=Sunday..6=Saturday; Django week_day is 1=Sunday..7=Saturday.
                DatePart::WeekDay => format!("(CAST(strftime('%w', {col}) AS INTEGER) + 1)"),
            },
            Dialect::MySql => match part {
                DatePart::Year => format!("EXTRACT(YEAR FROM {col})"),
                DatePart::Month => format!("EXTRACT(MONTH FROM {col})"),
                DatePart::Day => format!("EXTRACT(DAY FROM {col})"),
                DatePart::Hour => format!("EXTRACT(HOUR FROM {col})"),
                // MySQL DAYOFWEEK is already 1=Sunday..7=Saturday.
                DatePart::WeekDay => format!("DAYOFWEEK({col})"),
            },
        }
    }

    /// Returns the binary blob data type name for DDL.
    ///
    /// `BYTEA` for Postgres; `BLOB` for SQLite; `LONGBLOB` for MySQL.
    pub fn bytea_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "BYTEA",
            Dialect::Sqlite => "BLOB",
            Dialect::MySql => "LONGBLOB",
        }
    }

    /// Returns the column type string for an auto-incrementing integer primary key.
    ///
    /// Postgres: `SERIAL PRIMARY KEY`; SQLite: `INTEGER PRIMARY KEY AUTOINCREMENT`;
    /// MySQL: `BIGINT AUTO_INCREMENT PRIMARY KEY`.
    pub fn auto_pk_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "SERIAL PRIMARY KEY",
            Dialect::Sqlite => "INTEGER PRIMARY KEY AUTOINCREMENT",
            Dialect::MySql => "BIGINT AUTO_INCREMENT PRIMARY KEY",
        }
    }

    /// Returns the column type string for a 64-bit integer.
    pub fn bigint_type(&self) -> &'static str {
        match self {
            Dialect::Postgres | Dialect::MySql => "BIGINT",
            Dialect::Sqlite => "INTEGER",
        }
    }

    /// Returns the column type string for a boolean.
    pub fn bool_type(&self) -> &'static str {
        match self {
            Dialect::Postgres | Dialect::Sqlite | Dialect::MySql => "BOOLEAN",
        }
    }

    /// Returns the column type string for a timezone-aware timestamp.
    pub fn timestamp_type(&self) -> &'static str {
        match self {
            Dialect::Postgres => "TIMESTAMPTZ",
            Dialect::Sqlite => "TEXT",
            Dialect::MySql => "DATETIME(6)",
        }
    }

    /// Returns the SQL expression string for the current timestamp.
    pub fn current_timestamp(&self) -> &'static str {
        match self {
            Dialect::Postgres => "now()",
            Dialect::Sqlite => "CURRENT_TIMESTAMP",
            Dialect::MySql => "CURRENT_TIMESTAMP(6)",
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
        } else if url.starts_with("mysql://") || url.starts_with("mariadb://") {
            Dialect::MySql
        } else {
            Dialect::Postgres
        }
    }
}
