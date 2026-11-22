use crate::error::MigrationError;
use djangors_db::Dialect;
use djangors_orm::{FieldKind, FieldMeta};

/// Maps an ORM [`FieldMeta`] definition to its corresponding SQL type string.
pub fn field_meta_to_sql_type(
    field: &FieldMeta,
    dialect: Dialect,
) -> Result<String, MigrationError> {
    sql_type_for(
        &field.kind,
        field.max_length,
        field.auto,
        field.name,
        dialect,
    )
}

/// Maps a field's kind/max_length/auto to its corresponding SQL type string for a given dialect.
pub fn sql_type_for(
    kind: &FieldKind,
    max_length: Option<u32>,
    auto: bool,
    field_name: &str,
    dialect: Dialect,
) -> Result<String, MigrationError> {
    match dialect {
        Dialect::Postgres => match kind {
            FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
                if let Some(n) = max_length {
                    Ok(format!("VARCHAR({})", n))
                } else {
                    Err(MigrationError::MissingMaxLength {
                        field: field_name.to_string(),
                    })
                }
            }
            FieldKind::Text | FieldKind::FileField => Ok("TEXT".to_string()),
            FieldKind::Integer => {
                if auto {
                    Ok("SERIAL".to_string())
                } else {
                    Ok("INTEGER".to_string())
                }
            }
            FieldKind::BigInt => {
                if auto {
                    Ok("BIGSERIAL".to_string())
                } else {
                    Ok("BIGINT".to_string())
                }
            }
            FieldKind::Float => Ok("DOUBLE PRECISION".to_string()),
            FieldKind::Decimal {
                max_digits,
                decimal_places,
            } => Ok(format!("NUMERIC({}, {})", max_digits, decimal_places)),
            FieldKind::Boolean => Ok("BOOLEAN".to_string()),
            FieldKind::Date => Ok("DATE".to_string()),
            FieldKind::DateTime => Ok("TIMESTAMPTZ".to_string()),
            FieldKind::Time => Ok("TIME".to_string()),
            FieldKind::Duration => Ok("INTERVAL".to_string()),
            FieldKind::Uuid => Ok("UUID".to_string()),
            FieldKind::Ip => Ok("INET".to_string()),
            FieldKind::Binary => Ok("BYTEA".to_string()),
            FieldKind::Json => Ok("JSONB".to_string()),
        },
        Dialect::Sqlite => match kind {
            FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
                Ok("TEXT".to_string())
            }
            FieldKind::Text | FieldKind::FileField => Ok("TEXT".to_string()),
            FieldKind::Integer | FieldKind::BigInt => {
                if auto {
                    Ok("INTEGER PRIMARY KEY AUTOINCREMENT".to_string())
                } else {
                    Ok("INTEGER".to_string())
                }
            }
            FieldKind::Float => Ok("REAL".to_string()),
            FieldKind::Decimal { .. } => Ok("NUMERIC".to_string()),
            FieldKind::Boolean => Ok("INTEGER".to_string()),
            FieldKind::Date
            | FieldKind::DateTime
            | FieldKind::Time
            | FieldKind::Duration
            | FieldKind::Uuid
            | FieldKind::Ip
            | FieldKind::Json => Ok("TEXT".to_string()),
            FieldKind::Binary => Ok("BLOB".to_string()),
        },
        Dialect::MySql => match kind {
            FieldKind::Char | FieldKind::Email | FieldKind::Url | FieldKind::Slug => {
                if let Some(n) = max_length {
                    Ok(format!("VARCHAR({})", n))
                } else {
                    Err(MigrationError::MissingMaxLength {
                        field: field_name.to_string(),
                    })
                }
            }
            FieldKind::Text | FieldKind::FileField => Ok("LONGTEXT".to_string()),
            FieldKind::Integer => {
                if auto {
                    Ok("INT AUTO_INCREMENT".to_string())
                } else {
                    Ok("INT".to_string())
                }
            }
            FieldKind::BigInt => {
                if auto {
                    Ok("BIGINT AUTO_INCREMENT".to_string())
                } else {
                    Ok("BIGINT".to_string())
                }
            }
            FieldKind::Float => Ok("DOUBLE".to_string()),
            FieldKind::Decimal {
                max_digits,
                decimal_places,
            } => Ok(format!("DECIMAL({}, {})", max_digits, decimal_places)),
            FieldKind::Boolean => Ok("BOOLEAN".to_string()),
            FieldKind::Date => Ok("DATE".to_string()),
            FieldKind::DateTime => Ok("DATETIME(6)".to_string()),
            FieldKind::Time => Ok("TIME(6)".to_string()),
            FieldKind::Duration => Ok("BIGINT".to_string()),
            FieldKind::Uuid => Ok("CHAR(36)".to_string()),
            FieldKind::Ip => Ok("VARCHAR(45)".to_string()),
            FieldKind::Binary => Ok("LONGBLOB".to_string()),
            FieldKind::Json => Ok("JSON".to_string()),
        },
    }
}

#[cfg(test)]
#[path = "type_mapping_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "type_mapping_sqlite_ddl_tests.rs"]
mod sqlite_ddl_tests;
