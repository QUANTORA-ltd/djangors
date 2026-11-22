//! Multi-database routing (Django `DATABASE_ROUTERS` equivalent).

use std::collections::HashMap;
use std::sync::Arc;

use crate::database::Database;
use crate::error::DbError;

/// Decides which database alias handles reads, writes, and migrations.
///
/// Mirrors Django's `DATABASE_ROUTERS` protocol.
pub trait DatabaseRouter: Send + Sync {
    /// Returns the database alias to use for read queries against `model_table`,
    /// or `None` to defer to the next router / default database.
    fn db_for_read(&self, model_table: &str) -> Option<&str>;

    /// Returns the database alias to use for write queries against `model_table`,
    /// or `None` to defer to the next router / default database.
    fn db_for_write(&self, model_table: &str) -> Option<&str>;

    /// Returns whether migrations for `app_label` are allowed on `db_alias`.
    fn allow_migrate(&self, db_alias: &str, app_label: &str) -> Option<bool> {
        let _ = (db_alias, app_label);
        None
    }
}

/// Built-in primary/replica router that sends writes to `primary` and reads to `replica`.
#[derive(Debug, Clone)]
pub struct PrimaryReplicaRouter {
    primary: String,
    replica: String,
}

impl PrimaryReplicaRouter {
    /// Creates a router directing writes to `primary` and reads to `replica`.
    pub fn new(primary: impl Into<String>, replica: impl Into<String>) -> Self {
        Self {
            primary: primary.into(),
            replica: replica.into(),
        }
    }
}

impl DatabaseRouter for PrimaryReplicaRouter {
    fn db_for_read(&self, _model_table: &str) -> Option<&str> {
        Some(&self.replica)
    }

    fn db_for_write(&self, _model_table: &str) -> Option<&str> {
        Some(&self.primary)
    }

    fn allow_migrate(&self, db_alias: &str, _app_label: &str) -> Option<bool> {
        Some(db_alias == self.primary)
    }
}

/// Container holding multiple named [`Database`] pools and ordered [`DatabaseRouter`]s.
#[derive(Clone, Default)]
pub struct MultiDatabase {
    databases: HashMap<String, Database>,
    routers: Vec<Arc<dyn DatabaseRouter>>,
    default_alias: String,
}

impl MultiDatabase {
    /// Creates a new `MultiDatabase` with the given `default` database handle.
    pub fn new(default: Database) -> Self {
        let mut databases = HashMap::new();
        databases.insert("default".to_string(), default);
        Self {
            databases,
            routers: Vec::new(),
            default_alias: "default".to_string(),
        }
    }

    /// Registers an additional named database pool (e.g. `"replica"`, `"analytics"`).
    pub fn register(mut self, alias: impl Into<String>, db: Database) -> Self {
        self.databases.insert(alias.into(), db);
        self
    }

    /// Appends a [`DatabaseRouter`] to the routing chain.
    pub fn with_router(mut self, router: impl DatabaseRouter + 'static) -> Self {
        self.routers.push(Arc::new(router));
        self
    }

    /// Looks up a registered database pool by alias.
    pub fn get(&self, alias: &str) -> Result<&Database, DbError> {
        self.databases
            .get(alias)
            .ok_or_else(|| DbError::ConnectionFailed(format!("unknown database alias `{alias}`")))
    }

    /// Returns the default database pool.
    pub fn default_db(&self) -> Result<&Database, DbError> {
        self.get(&self.default_alias)
    }

    /// Resolves the database pool to use for a read query on `model_table`.
    pub fn read_db(&self, model_table: &str) -> Result<&Database, DbError> {
        for r in &self.routers {
            if let Some(alias) = r.db_for_read(model_table) {
                return self.get(alias);
            }
        }
        self.default_db()
    }

    /// Resolves the database pool to use for a write query on `model_table`.
    pub fn write_db(&self, model_table: &str) -> Result<&Database, DbError> {
        for r in &self.routers {
            if let Some(alias) = r.db_for_write(model_table) {
                return self.get(alias);
            }
        }
        self.default_db()
    }

    /// Returns whether migrations for `app_label` are allowed on `db_alias`.
    pub fn allow_migrate(&self, db_alias: &str, app_label: &str) -> bool {
        for r in &self.routers {
            if let Some(allowed) = r.allow_migrate(db_alias, app_label) {
                return allowed;
            }
        }
        true
    }
}
