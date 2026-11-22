use crate::error::MigrationError;
use djangors_db::{BindValue, Dialect};
use djangors_orm::ConstraintMeta;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// Reference target for a foreign key constraint.
#[derive(Debug, Clone)]
pub struct ForeignKeyRef {
    /// Target table name.
    pub table: String,
    /// Target column name.
    pub column: String,
    /// Foreign key `ON DELETE` clause string (`"CASCADE"`, `"RESTRICT"`, etc.).
    pub on_delete: String,
}

/// Definition of a database table column for migration operations.
#[derive(Debug, Clone)]
pub struct ColumnDef {
    /// Column name in the database.
    pub name: String,
    /// Whether the column allows NULL values.
    pub nullable: bool,
    /// SQL type definition string (e.g., `VARCHAR(255)`).
    pub sql_type: String,
    /// Whether the column is part of the primary key.
    pub primary_key: bool,
    /// Whether the column has a UNIQUE constraint.
    pub unique: bool,
    /// Optional default SQL expression.
    pub default_sql: Option<String>,
    /// Optional foreign key constraint reference.
    pub references: Option<ForeignKeyRef>,
}

/// A DDL operation step in a migration plan.
#[derive(Debug, Clone)]
pub enum Operation {
    /// Creates a table with the specified name and columns.
    CreateTable {
        /// Name of the table to create.
        table_name: String,
        /// List of column definitions for the table.
        columns: Vec<ColumnDef>,
        /// Raw SQL CHECK constraint expressions.
        check_constraints: Vec<String>,
    },
    /// Adds a column to a table.
    AddColumn {
        /// Name of the table to alter.
        table_name: String,
        /// Definition of the column to add.
        column: ColumnDef,
    },
    /// Drops a column (not mechanically reversible).
    DropColumn {
        /// Name of the table to alter.
        table_name: String,
        /// Name of the column to drop.
        column_name: String,
    },
    /// Changes a column type (not mechanically reversible).
    AlterColumnType {
        /// Name of the table to alter.
        table_name: String,
        /// Name of the column to alter.
        column_name: String,
        /// New SQL data type definition.
        new_sql_type: String,
    },
    /// Renames a column.
    RenameColumn {
        /// Name of the table to alter.
        table_name: String,
        /// Existing column name.
        old_name: String,
        /// New column name.
        new_name: String,
    },
    /// Renames a table (Django `RenameModel` equivalent).
    RenameTable {
        /// Existing table name.
        old_name: String,
        /// New table name.
        new_name: String,
    },
    /// Drops a table.
    DropTable {
        /// Name of the table to drop.
        table_name: String,
    },
    /// Creates an index on a table.
    CreateIndex {
        /// Name of the index.
        index_name: String,
        /// Name of the table the index is on.
        table_name: String,
        /// Column names to index.
        columns: Vec<String>,
    },
    /// Drops an index.
    DropIndex {
        /// Name of the index to drop.
        index_name: String,
    },
    /// Adds a table-level constraint (`CHECK`, `UNIQUE`, partial unique index, or `EXCLUDE`).
    AddConstraint {
        /// Name of the table to add the constraint to.
        table_name: String,
        /// Declarative constraint specification.
        constraint: ConstraintMeta,
    },
    /// Removes a table-level constraint or backing unique index.
    RemoveConstraint {
        /// Name of the table containing the constraint.
        table_name: String,
        /// Name of the constraint (or partial unique index) to drop.
        constraint_name: String,
    },
    /// Alters whether a column permits `NULL` values.
    AlterColumnNullability {
        /// Name of the table to alter.
        table_name: String,
        /// Name of the column to alter.
        column_name: String,
        /// `true` to allow `NULL` (`DROP NOT NULL`), `false` to require non-null (`SET NOT NULL`).
        nullable: bool,
    },
    /// Sets or drops a column's default SQL expression.
    AlterColumnDefault {
        /// Name of the table to alter.
        table_name: String,
        /// Name of the column to alter.
        column_name: String,
        /// `Some(expr)` to `SET DEFAULT <expr>`, or `None` to `DROP DEFAULT`.
        default_sql: Option<String>,
    },
    /// Executes arbitrary raw SQL (Django `RunSQL` equivalent).
    RunSql {
        /// Forward SQL statement(s).
        sql: String,
        /// Optional reverse SQL statement(s) making this operation invertible.
        reverse_sql: Option<String>,
    },
}

impl Operation {
    /// Generates the SQL statement for this migration operation.
    pub fn to_sql(&self, dialect: Dialect) -> Result<String, MigrationError> {
        match self {
            Operation::CreateTable {
                table_name,
                columns,
                check_constraints,
            } => {
                let col_sqls: Vec<String> = columns.iter().map(column_sql).collect();
                let mut sql = format!(
                    "CREATE TABLE IF NOT EXISTS \"{}\" (\n    {}\n",
                    table_name,
                    col_sqls.join(",\n    ")
                );
                if !check_constraints.is_empty() {
                    for ck in check_constraints {
                        sql.push_str(&format!(",\n    {}", ck));
                    }
                }
                sql.push_str("\n);");
                Ok(sql)
            }
            Operation::AddColumn { table_name, column } => Ok(format!(
                "ALTER TABLE \"{}\" ADD COLUMN {};",
                table_name,
                column_sql(column)
            )),
            Operation::DropColumn {
                table_name,
                column_name,
            } => Ok(format!(
                "ALTER TABLE \"{}\" DROP COLUMN \"{}\";",
                table_name, column_name
            )),
            Operation::AlterColumnType {
                table_name,
                column_name,
                new_sql_type,
            } => match dialect {
                Dialect::Postgres => Ok(format!(
                    "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" TYPE {} USING \"{}\"::{};",
                    table_name, column_name, new_sql_type, column_name, new_sql_type
                )),
                Dialect::MySql => Ok(format!(
                    "ALTER TABLE `{}` MODIFY COLUMN `{}` {};",
                    table_name, column_name, new_sql_type
                )),
                Dialect::Sqlite => Err(MigrationError::UnsupportedOnDialect {
                    operation: "AlterColumnType".to_string(),
                    dialect: "Sqlite".to_string(),
                }),
            },
            Operation::RenameColumn {
                table_name,
                old_name,
                new_name,
            } => Ok(format!(
                "ALTER TABLE \"{}\" RENAME COLUMN \"{}\" TO \"{}\";",
                table_name, old_name, new_name
            )),
            Operation::RenameTable { old_name, new_name } => Ok(format!(
                "ALTER TABLE {} RENAME TO {};",
                dialect.quote_ident(old_name),
                dialect.quote_ident(new_name)
            )),
            Operation::DropTable { table_name } => Ok(format!("DROP TABLE \"{}\";", table_name)),
            Operation::CreateIndex {
                index_name,
                table_name,
                columns,
            } => {
                let cols: Vec<String> = columns.iter().map(|c| format!("\"{}\"", c)).collect();
                Ok(format!(
                    "CREATE INDEX IF NOT EXISTS \"{}\" ON \"{}\" ({});",
                    index_name,
                    table_name,
                    cols.join(", ")
                ))
            }
            Operation::DropIndex { index_name } => {
                Ok(format!("DROP INDEX IF EXISTS \"{}\";", index_name))
            }
            Operation::AddConstraint {
                table_name,
                constraint,
            } => {
                let tbl = dialect.quote_ident(table_name);
                match constraint {
                    ConstraintMeta::Check { name, expression } => match dialect {
                        Dialect::Postgres | Dialect::MySql => Ok(format!(
                            "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({});",
                            tbl,
                            dialect.quote_ident(name),
                            expression
                        )),
                        Dialect::Sqlite => Err(MigrationError::UnsupportedOnDialect {
                            operation: "AddConstraint(Check)".to_string(),
                            dialect: "Sqlite".to_string(),
                        }),
                    },
                    ConstraintMeta::Unique {
                        name,
                        fields,
                        condition,
                    } => {
                        let cols: Vec<String> =
                            fields.iter().map(|f| dialect.quote_ident(f)).collect();
                        let cname = dialect.quote_ident(name);
                        if let Some(cond) = condition {
                            match dialect {
                                Dialect::Postgres | Dialect::Sqlite => Ok(format!(
                                    "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {} ({}) WHERE {};",
                                    cname,
                                    tbl,
                                    cols.join(", "),
                                    cond
                                )),
                                Dialect::MySql => Err(MigrationError::UnsupportedOnDialect {
                                    operation: "AddConstraint(PartialUnique)".to_string(),
                                    dialect: "MySql".to_string(),
                                }),
                            }
                        } else {
                            match dialect {
                                Dialect::Postgres | Dialect::MySql => Ok(format!(
                                    "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
                                    tbl,
                                    cname,
                                    cols.join(", ")
                                )),
                                Dialect::Sqlite => Ok(format!(
                                    "CREATE UNIQUE INDEX IF NOT EXISTS {} ON {} ({});",
                                    cname,
                                    tbl,
                                    cols.join(", ")
                                )),
                            }
                        }
                    }
                    ConstraintMeta::Exclusion {
                        name,
                        index_method,
                        expressions,
                        condition,
                    } => match dialect {
                        Dialect::Postgres => {
                            let cname = dialect.quote_ident(name);
                            let mut sql = format!(
                                "ALTER TABLE {} ADD CONSTRAINT {} EXCLUDE USING {} ({})",
                                tbl, cname, index_method, expressions
                            );
                            if let Some(cond) = condition {
                                sql.push_str(&format!(" WHERE ({})", cond));
                            }
                            sql.push(';');
                            Ok(sql)
                        }
                        Dialect::Sqlite => Err(MigrationError::UnsupportedOnDialect {
                            operation: "AddConstraint(Exclusion)".to_string(),
                            dialect: "Sqlite".to_string(),
                        }),
                        Dialect::MySql => Err(MigrationError::UnsupportedOnDialect {
                            operation: "AddConstraint(Exclusion)".to_string(),
                            dialect: "MySql".to_string(),
                        }),
                    },
                }
            }
            Operation::RemoveConstraint {
                table_name,
                constraint_name,
            } => match dialect {
                Dialect::Postgres => Ok(format!(
                    "ALTER TABLE \"{}\" DROP CONSTRAINT IF EXISTS \"{}\";",
                    table_name, constraint_name
                )),
                Dialect::Sqlite => Ok(format!("DROP INDEX IF EXISTS \"{}\";", constraint_name)),
                Dialect::MySql => Ok(format!(
                    "ALTER TABLE `{}` DROP CONSTRAINT `{}`;",
                    table_name, constraint_name
                )),
            },
            Operation::AlterColumnNullability {
                table_name,
                column_name,
                nullable,
            } => match dialect {
                Dialect::Postgres => {
                    let action = if *nullable {
                        "DROP NOT NULL"
                    } else {
                        "SET NOT NULL"
                    };
                    Ok(format!(
                        "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" {};",
                        table_name, column_name, action
                    ))
                }
                Dialect::Sqlite => Err(MigrationError::UnsupportedOnDialect {
                    operation: "AlterColumnNullability".to_string(),
                    dialect: "Sqlite".to_string(),
                }),
                Dialect::MySql => Err(MigrationError::UnsupportedOnDialect {
                    operation: "AlterColumnNullability".to_string(),
                    dialect: "MySql".to_string(),
                }),
            },
            Operation::AlterColumnDefault {
                table_name,
                column_name,
                default_sql,
            } => match dialect {
                Dialect::Postgres => match default_sql {
                    Some(expr) => Ok(format!(
                        "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" SET DEFAULT {};",
                        table_name, column_name, expr
                    )),
                    None => Ok(format!(
                        "ALTER TABLE \"{}\" ALTER COLUMN \"{}\" DROP DEFAULT;",
                        table_name, column_name
                    )),
                },
                Dialect::MySql => match default_sql {
                    Some(expr) => Ok(format!(
                        "ALTER TABLE `{}` ALTER COLUMN `{}` SET DEFAULT {};",
                        table_name, column_name, expr
                    )),
                    None => Ok(format!(
                        "ALTER TABLE `{}` ALTER COLUMN `{}` DROP DEFAULT;",
                        table_name, column_name
                    )),
                },
                Dialect::Sqlite => Err(MigrationError::UnsupportedOnDialect {
                    operation: "AlterColumnDefault".to_string(),
                    dialect: "Sqlite".to_string(),
                }),
            },
            Operation::RunSql { sql, .. } => Ok(sql.clone()),
        }
    }

    /// Returns the mechanically safe reverse operation, if one exists.
    pub fn reverse(&self) -> Option<Self> {
        match self {
            Self::CreateTable { table_name, .. } => Some(Self::DropTable {
                table_name: table_name.clone(),
            }),
            Self::AddColumn { table_name, column } => Some(Self::DropColumn {
                table_name: table_name.clone(),
                column_name: column.name.clone(),
            }),
            Self::RenameColumn {
                table_name,
                old_name,
                new_name,
            } => Some(Self::RenameColumn {
                table_name: table_name.clone(),
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            }),
            Self::RenameTable { old_name, new_name } => Some(Self::RenameTable {
                old_name: new_name.clone(),
                new_name: old_name.clone(),
            }),
            Self::CreateIndex { index_name, .. } => Some(Self::DropIndex {
                index_name: index_name.clone(),
            }),
            Self::AddConstraint {
                table_name,
                constraint,
            } => {
                let constraint_name = match constraint {
                    ConstraintMeta::Check { name, .. }
                    | ConstraintMeta::Unique { name, .. }
                    | ConstraintMeta::Exclusion { name, .. } => name.clone(),
                };
                Some(Self::RemoveConstraint {
                    table_name: table_name.clone(),
                    constraint_name,
                })
            }
            Self::AlterColumnNullability {
                table_name,
                column_name,
                nullable,
            } => Some(Self::AlterColumnNullability {
                table_name: table_name.clone(),
                column_name: column_name.clone(),
                nullable: !*nullable,
            }),
            Self::RunSql { sql, reverse_sql } => reverse_sql.as_ref().map(|rev| Self::RunSql {
                sql: rev.clone(),
                reverse_sql: Some(sql.clone()),
            }),
            Self::DropColumn { .. }
            | Self::AlterColumnType { .. }
            | Self::DropTable { .. }
            | Self::DropIndex { .. }
            | Self::RemoveConstraint { .. }
            | Self::AlterColumnDefault { .. } => None,
        }
    }

    /// Generates reverse SQL when this operation is safely invertible.
    pub fn to_down_sql(&self, dialect: Dialect) -> Option<Result<String, MigrationError>> {
        self.reverse().map(|op| op.to_sql(dialect))
    }
}

/// Boxed future returned by async [`DataMigration`] methods.
pub type MigrationFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), MigrationError>> + Send + 'a>>;

/// Function pointer signature used by [`RunRust`].
pub type DataMigrationFn = for<'a, 'c> fn(&'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a>;

/// A Rust-authored data migration (Django `RunPython` equivalent) that executes
/// inside a database transaction and is tracked in `djangors_migrations`.
pub trait DataMigration: Send + Sync {
    /// Unique migration name recorded in `djangors_migrations`.
    fn name(&self) -> &'static str;

    /// Forward data migration logic executed inside a transaction.
    fn run_up<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a>;

    /// Optional reverse data migration logic executed during rollback.
    ///
    /// Defaults to returning [`MigrationError::NonInvertible`].
    fn run_down<'a, 'c>(&'a self, _conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        let name = self.name().to_string();
        Box::pin(async move { Err(MigrationError::NonInvertible { name }) })
    }
}

impl<T: DataMigration + ?Sized> DataMigration for &'static T {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn run_up<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        (**self).run_up(conn)
    }

    fn run_down<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        (**self).run_down(conn)
    }
}

impl<T: DataMigration + ?Sized> DataMigration for Arc<T> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn run_up<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        (**self).run_up(conn)
    }

    fn run_down<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        (**self).run_down(conn)
    }
}

/// Function-pointer-backed [`DataMigration`] (Django `RunPython` equivalent).
#[derive(Clone, Copy)]
pub struct RunRust {
    /// Unique migration name recorded in `djangors_migrations`.
    pub name: &'static str,
    /// Forward migration function executed inside a transaction.
    pub up: DataMigrationFn,
    /// Optional reverse migration function executed on rollback.
    pub down: Option<DataMigrationFn>,
}

impl std::fmt::Debug for RunRust {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRust")
            .field("name", &self.name)
            .field("reversible", &self.down.is_some())
            .finish()
    }
}

impl RunRust {
    /// Creates a new forward-only [`RunRust`] data migration.
    pub const fn new(name: &'static str, up: DataMigrationFn) -> Self {
        Self {
            name,
            up,
            down: None,
        }
    }

    /// Attaches a reverse migration function, making this [`RunRust`] invertible.
    pub const fn with_reverse(mut self, down: DataMigrationFn) -> Self {
        self.down = Some(down);
        self
    }
}

impl DataMigration for RunRust {
    fn name(&self) -> &'static str {
        self.name
    }

    fn run_up<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        (self.up)(conn)
    }

    fn run_down<'a, 'c>(&'a self, conn: &'a mut djangors_db::Conn<'c>) -> MigrationFuture<'a> {
        match self.down {
            Some(down) => down(conn),
            None => {
                let name = self.name.to_string();
                Box::pin(async move { Err(MigrationError::NonInvertible { name }) })
            }
        }
    }
}

/// Applies a [`DataMigration`] inside a transaction and records it in `djangors_migrations`.
///
/// This operation is idempotent: if `migration.name()` is already recorded in
/// `djangors_migrations`, it returns `Ok(())` without re-running `run_up`.
pub async fn apply_data_migration<M: DataMigration + 'static>(
    db: &djangors_db::Database,
    migration: M,
) -> Result<(), MigrationError> {
    crate::ensure_history(db).await?;
    let name = migration.name();
    let dialect = db.dialect();
    let query = format!(
        "SELECT 1 FROM djangors_migrations WHERE name = {}",
        dialect.placeholder(1)
    );
    let already_applied = db
        .conn()
        .fetch_optional(&query, &[BindValue::Text(name.to_string())])
        .await?
        .is_some();
    if already_applied {
        return Ok(());
    }

    let captured_err: Arc<Mutex<Option<MigrationError>>> = Arc::new(Mutex::new(None));
    let captured_clone = Arc::clone(&captured_err);
    let res = db
        .transaction_conn(move |conn| {
            Box::pin(async move {
                if let Err(err) = migration.run_up(conn).await {
                    *captured_clone.lock().unwrap() = Some(err);
                    return Err(djangors_db::DbError::TransactionFailed(format!(
                        "data migration '{}' failed",
                        name
                    )));
                }
                let dialect = conn.dialect();
                let insert_sql = format!(
                    "INSERT INTO djangors_migrations (name) VALUES ({})",
                    dialect.placeholder(1)
                );
                conn.execute(&insert_sql, &[BindValue::Text(name.to_string())])
                    .await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await;

    if let Some(err) = captured_err.lock().unwrap().take() {
        return Err(err);
    }
    res.map_err(MigrationError::Database)
}

/// Rolls back a previously applied [`DataMigration`] inside a transaction and removes
/// its record from `djangors_migrations`.
///
/// If `migration.name()` is not currently recorded in `djangors_migrations`, this is a no-op.
pub async fn rollback_data_migration<M: DataMigration + 'static>(
    db: &djangors_db::Database,
    migration: M,
) -> Result<(), MigrationError> {
    crate::ensure_history(db).await?;
    let name = migration.name();
    let dialect = db.dialect();
    let query = format!(
        "SELECT 1 FROM djangors_migrations WHERE name = {}",
        dialect.placeholder(1)
    );
    let applied = db
        .conn()
        .fetch_optional(&query, &[BindValue::Text(name.to_string())])
        .await?
        .is_some();
    if !applied {
        return Ok(());
    }

    let captured_err: Arc<Mutex<Option<MigrationError>>> = Arc::new(Mutex::new(None));
    let captured_clone = Arc::clone(&captured_err);
    let res = db
        .transaction_conn(move |conn| {
            Box::pin(async move {
                if let Err(err) = migration.run_down(conn).await {
                    *captured_clone.lock().unwrap() = Some(err);
                    return Err(djangors_db::DbError::TransactionFailed(format!(
                        "data migration rollback '{}' failed",
                        name
                    )));
                }
                let dialect = conn.dialect();
                let delete_sql = format!(
                    "DELETE FROM djangors_migrations WHERE name = {}",
                    dialect.placeholder(1)
                );
                conn.execute(&delete_sql, &[BindValue::Text(name.to_string())])
                    .await?;
                Ok::<(), djangors_db::DbError>(())
            })
        })
        .await;

    if let Some(err) = captured_err.lock().unwrap().take() {
        return Err(err);
    }
    res.map_err(MigrationError::Database)
}

fn column_sql(col: &ColumnDef) -> String {
    let mut sql = format!("\"{}\" {}", col.name, col.sql_type);
    if col.sql_type.contains("PRIMARY KEY") {
        if let Some(default) = &col.default_sql {
            sql.push_str(&format!(" DEFAULT {}", default));
        }
        if let Some(refs) = &col.references {
            sql.push_str(&format!(
                " REFERENCES \"{}\"(\"{}\") ON DELETE {}",
                refs.table, refs.column, refs.on_delete
            ));
        }
        return sql;
    }
    if !col.nullable {
        sql.push_str(" NOT NULL");
    }
    if col.primary_key {
        sql.push_str(" PRIMARY KEY");
    }
    if col.unique && !col.primary_key {
        sql.push_str(" UNIQUE");
    }
    if let Some(default) = &col.default_sql {
        sql.push_str(&format!(" DEFAULT {}", default));
    }
    if let Some(refs) = &col.references {
        sql.push_str(&format!(
            " REFERENCES \"{}\"(\"{}\") ON DELETE {}",
            refs.table, refs.column, refs.on_delete
        ));
    }
    sql
}
