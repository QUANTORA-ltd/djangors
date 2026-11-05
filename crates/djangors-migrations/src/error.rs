use thiserror::Error;

/// Error types for database migration planning and execution.
#[derive(Error, Debug)]
pub enum MigrationError {
    /// A character field is missing a required `max_length` attribute.
    #[error("Field {field} is missing max_length attribute")]
    MissingMaxLength {
        /// The name of the model field.
        field: String,
    },

    /// A cyclic dependency loop was detected between model foreign keys.
    #[error("Cyclic dependency detected between models: {models:?}")]
    CyclicDependency {
        /// Model names involved in the dependency cycle.
        models: Vec<String>,
    },

    /// A database operation or connection error occurred.
    #[error("Database error: {0}")]
    Database(#[from] djangors_db::DbError),

    /// A raw SQL query execution error occurred.
    #[error("Query execution error: {0}")]
    Query(#[from] sqlx::Error),

    /// A migration file could not be read.
    #[error("migration file error: {0}")]
    Io(#[from] std::io::Error),

    /// A migration cannot be safely rolled back.
    #[error("migration {name} has no down SQL and cannot be rolled back")]
    NonInvertible {
        /// Migration filename.
        name: String,
    },

    /// The database schema does not contain tables or columns required by registered models.
    #[error(
        "database schema is behind model table {table}; missing {missing:?}. Commit migrations with `dj makemigrations` and apply them with `dj migrate`"
    )]
    SchemaBehind {
        /// Model table whose schema is incomplete.
        table: String,
        /// Missing table or column names.
        missing: Vec<String>,
    },

    /// An operation is not supported on the specified SQL dialect.
    #[error("operation {operation} is not supported on dialect {dialect}")]
    UnsupportedOnDialect {
        /// Description of the unsupported operation.
        operation: String,
        /// Name of the SQL dialect.
        dialect: String,
    },
}
