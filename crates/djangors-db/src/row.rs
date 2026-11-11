//! Database row wrapper across backends.

use sqlx::Row;

/// Wrapper over backend-specific row types.
pub enum DbRow {
    /// PostgreSQL row.
    #[cfg(feature = "postgres")]
    Pg(sqlx::postgres::PgRow),
    /// SQLite row.
    #[cfg(feature = "sqlite")]
    Sqlite(sqlx::sqlite::SqliteRow),
}

impl std::fmt::Debug for DbRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(_) => write!(f, "DbRow::Pg(...)"),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(_) => write!(f, "DbRow::Sqlite(...)"),
        }
    }
}

impl DbRow {
    /// Try decoding an optional i64 column by index.
    pub fn try_i64(&self, idx: usize) -> Result<Option<i64>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r
                .try_get::<Option<i64>, _>(idx)
                .or_else(|_| {
                    r.try_get::<Option<i32>, _>(idx)
                        .map(|opt| opt.map(|v| v as i64))
                })
                .or_else(|_| {
                    r.try_get::<Option<i16>, _>(idx)
                        .map(|opt| opt.map(|v| v as i64))
                }),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r
                .try_get::<Option<i64>, _>(idx)
                .or_else(|_| {
                    r.try_get::<Option<i32>, _>(idx)
                        .map(|opt| opt.map(|v| v as i64))
                })
                .or_else(|_| {
                    r.try_get::<Option<i16>, _>(idx)
                        .map(|opt| opt.map(|v| v as i64))
                }),
        }
    }

    /// Try decoding an optional f64 column by index.
    pub fn try_f64(&self, idx: usize) -> Result<Option<f64>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get::<Option<f64>, _>(idx).or_else(|_| {
                r.try_get::<Option<f32>, _>(idx)
                    .map(|opt| opt.map(|v| v as f64))
            }),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get::<Option<f64>, _>(idx).or_else(|_| {
                r.try_get::<Option<f32>, _>(idx)
                    .map(|opt| opt.map(|v| v as f64))
            }),
        }
    }

    /// Try decoding an optional String column by index.
    pub fn try_string(&self, idx: usize) -> Result<Option<String>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(idx),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(idx),
        }
    }

    /// Try decoding an optional bool column by index.
    pub fn try_bool(&self, idx: usize) -> Result<Option<bool>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(idx),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(idx),
        }
    }

    /// Try decoding an optional Utc DateTime column by index.
    pub fn try_datetime(
        &self,
        idx: usize,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(idx),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(idx),
        }
    }

    /// Try decoding an optional byte vector column by index.
    pub fn try_bytes(&self, idx: usize) -> Result<Option<Vec<u8>>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(idx),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(idx),
        }
    }

    /// Try decoding an optional i64 column by name.
    pub fn try_i64_by_name(&self, name: &str) -> Result<Option<i64>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r
                .try_get::<Option<i64>, _>(name)
                .or_else(|_| {
                    r.try_get::<Option<i32>, _>(name)
                        .map(|opt| opt.map(|v| v as i64))
                })
                .or_else(|_| {
                    r.try_get::<Option<i16>, _>(name)
                        .map(|opt| opt.map(|v| v as i64))
                }),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r
                .try_get::<Option<i64>, _>(name)
                .or_else(|_| {
                    r.try_get::<Option<i32>, _>(name)
                        .map(|opt| opt.map(|v| v as i64))
                })
                .or_else(|_| {
                    r.try_get::<Option<i16>, _>(name)
                        .map(|opt| opt.map(|v| v as i64))
                }),
        }
    }

    /// Try decoding an optional f64 column by name.
    pub fn try_f64_by_name(&self, name: &str) -> Result<Option<f64>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get::<Option<f64>, _>(name).or_else(|_| {
                r.try_get::<Option<f32>, _>(name)
                    .map(|opt| opt.map(|v| v as f64))
            }),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get::<Option<f64>, _>(name).or_else(|_| {
                r.try_get::<Option<f32>, _>(name)
                    .map(|opt| opt.map(|v| v as f64))
            }),
        }
    }

    /// Try decoding an optional String column by name.
    pub fn try_string_by_name(&self, name: &str) -> Result<Option<String>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(name),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(name),
        }
    }

    /// Try decoding an optional bool column by name.
    pub fn try_bool_by_name(&self, name: &str) -> Result<Option<bool>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(name),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(name),
        }
    }

    /// Try decoding an optional Utc DateTime column by name.
    pub fn try_datetime_by_name(
        &self,
        name: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(name),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(name),
        }
    }

    /// Try decoding an optional byte vector column by name.
    pub fn try_bytes_by_name(&self, name: &str) -> Result<Option<Vec<u8>>, sqlx::Error> {
        match self {
            #[cfg(feature = "postgres")]
            DbRow::Pg(r) => r.try_get(name),
            #[cfg(feature = "sqlite")]
            DbRow::Sqlite(r) => r.try_get(name),
        }
    }
}
