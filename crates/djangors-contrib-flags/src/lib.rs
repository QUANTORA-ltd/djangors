#![deny(missing_docs)]
//! Database and cache-backed feature flags and gradual rollouts for Djangors.

use chrono::{DateTime, Utc};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_orm::Model as _;
use sha2::{Digest, Sha256};
#[cfg(feature = "cache")]
use std::sync::Arc;
#[cfg(feature = "cache")]
use std::time::Duration;
use thiserror::Error;

/// Errors produced by feature flag operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FlagError {
    /// Raw database error.
    #[error("database error: {0}")]
    Db(#[from] djangors_db::DbError),
    /// ORM query or persistence error.
    #[error("orm error: {0}")]
    Orm(#[from] djangors_orm::OrmError),
    /// JSON serialization or deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Cache backend error.
    #[cfg(feature = "cache")]
    #[error("cache error: {0}")]
    Cache(#[from] djangors_cache::CacheError),
}

/// Feature flag model persisted in `djangors_feature_flag`.
#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[djangors(app = "djangors_flags", table_name = "djangors_feature_flag")]
pub struct FeatureFlag {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Unique flag name.
    #[djangors(max_length = 128, unique)]
    pub name: String,
    /// Human-readable description of what the flag controls.
    pub description: String,
    /// Global kill switch; when `false`, the flag always evaluates to `false`.
    pub enabled: bool,
    /// Percentage of subjects (`0..=100`) that receive the flag when enabled.
    pub rollout_percentage: i32,
    /// JSON-encoded array of allowlisted user IDs.
    pub allowed_users: String,
    /// JSON-encoded array of allowlisted organization IDs.
    pub allowed_orgs: String,
    /// Timestamp of the last update (UTC).
    pub updated_at: DateTime<Utc>,
}

/// Evaluation context passed to [`FlagService::is_enabled`] and [`FlagService::evaluate`].
#[derive(Debug, Clone, Default)]
pub struct FlagContext {
    /// Optional user identifier for allowlist and rollout bucketing.
    pub user_id: Option<String>,
    /// Optional organization identifier for allowlist and fallback rollout bucketing.
    pub org_id: Option<String>,
}

impl FlagContext {
    /// Creates an empty [`FlagContext`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the user identifier on this context.
    pub fn user(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    /// Sets the organization identifier on this context.
    pub fn org(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }
}

/// Service for managing and evaluating [`FeatureFlag`] records.
#[derive(Clone)]
pub struct FlagService {
    db: Database,
    #[cfg(feature = "cache")]
    cache: Option<Arc<dyn djangors_cache::Cache>>,
    #[cfg(feature = "cache")]
    cache_ttl_secs: u64,
}

impl FlagService {
    /// Creates a new [`FlagService`] backed by `db`.
    pub fn new(db: Database) -> Self {
        Self {
            db,
            #[cfg(feature = "cache")]
            cache: None,
            #[cfg(feature = "cache")]
            cache_ttl_secs: 0,
        }
    }

    /// Attaches a [`djangors_cache::Cache`] backend with the given TTL in seconds.
    #[cfg(feature = "cache")]
    pub fn with_cache(mut self, cache: Arc<dyn djangors_cache::Cache>, ttl_secs: u64) -> Self {
        self.cache = Some(cache);
        self.cache_ttl_secs = ttl_secs;
        self
    }

    /// Evaluates whether `flag_name` is enabled for `ctx`.
    ///
    /// Returns `false` if the flag does not exist.
    pub async fn is_enabled(&self, flag_name: &str, ctx: &FlagContext) -> Result<bool, FlagError> {
        let flag = self.get_flag(flag_name).await?;
        Ok(flag.as_ref().is_some_and(|f| Self::evaluate(f, ctx)))
    }

    /// Pure deterministic evaluation of `flag` against `ctx`.
    ///
    /// 1. If `!flag.enabled`, returns `false`.
    /// 2. If `ctx.user_id` is in `flag.allowed_users` or `ctx.org_id` is in `flag.allowed_orgs`, returns `true`.
    /// 3. If `flag.rollout_percentage >= 100`, returns `true`.
    /// 4. If `flag.rollout_percentage <= 0`, returns `false`.
    /// 5. Otherwise, hashes `"{flag.name}:{subject}"` with SHA-256 (preferring `user_id`, falling back to `org_id`)
    ///    and checks `u32::from_be_bytes(digest[..4]) % 100 < flag.rollout_percentage as u32`.
    pub fn evaluate(flag: &FeatureFlag, ctx: &FlagContext) -> bool {
        if !flag.enabled {
            return false;
        }

        let allowed_users: Vec<String> =
            serde_json::from_str(&flag.allowed_users).unwrap_or_default();
        if ctx
            .user_id
            .as_ref()
            .is_some_and(|u| allowed_users.contains(u))
        {
            return true;
        }

        let allowed_orgs: Vec<String> =
            serde_json::from_str(&flag.allowed_orgs).unwrap_or_default();
        if ctx.org_id.as_ref().is_some_and(|o| allowed_orgs.contains(o)) {
            return true;
        }

        if flag.rollout_percentage >= 100 {
            return true;
        }
        if flag.rollout_percentage <= 0 {
            return false;
        }

        let Some(subject) = ctx.user_id.as_deref().or(ctx.org_id.as_deref()) else {
            return false;
        };

        let mut hasher = Sha256::new();
        hasher.update(flag.name.as_bytes());
        hasher.update(b":");
        hasher.update(subject.as_bytes());
        let digest = hasher.finalize();
        let bucket = u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 100;
        bucket < flag.rollout_percentage as u32
    }

    /// Creates or updates a feature flag and invalidates any cached entry.
    pub async fn set_flag(
        &self,
        name: &str,
        description: &str,
        enabled: bool,
        rollout_percentage: i32,
        allowed_users: &[&str],
        allowed_orgs: &[&str],
    ) -> Result<FeatureFlag, FlagError> {
        let allowed_users_json = serde_json::to_string(allowed_users)?;
        let allowed_orgs_json = serde_json::to_string(allowed_orgs)?;
        let now = Utc::now();

        let existing = FeatureFlag::objects()
            .filter(djangors_orm::q!(name = name))?
            .all(&self.db)
            .await?
            .into_iter()
            .next();

        let flag = if let Some(mut flag) = existing {
            flag.description = description.to_string();
            flag.enabled = enabled;
            flag.rollout_percentage = rollout_percentage;
            flag.allowed_users = allowed_users_json;
            flag.allowed_orgs = allowed_orgs_json;
            flag.updated_at = now;
            flag.update(&self.db).await?;
            flag
        } else {
            let flag = FeatureFlag {
                id: 0,
                name: name.to_string(),
                description: description.to_string(),
                enabled,
                rollout_percentage,
                allowed_users: allowed_users_json,
                allowed_orgs: allowed_orgs_json,
                updated_at: now,
            };
            flag.save(&self.db).await?
        };

        self.invalidate_cache(name).await?;
        Ok(flag)
    }

    /// Deletes a feature flag by name and invalidates any cached entry.
    ///
    /// Returns `true` if a flag existed and was deleted, or `false` otherwise.
    pub async fn delete_flag(&self, name: &str) -> Result<bool, FlagError> {
        let existing = FeatureFlag::objects()
            .filter(djangors_orm::q!(name = name))?
            .all(&self.db)
            .await?
            .into_iter()
            .next();

        let deleted = if let Some(flag) = existing {
            flag.delete(&self.db).await?;
            true
        } else {
            false
        };

        self.invalidate_cache(name).await?;
        Ok(deleted)
    }

    /// Returns all feature flags stored in the database.
    pub async fn list_flags(&self) -> Result<Vec<FeatureFlag>, FlagError> {
        Ok(FeatureFlag::objects().all(&self.db).await?)
    }

    async fn get_flag(&self, flag_name: &str) -> Result<Option<FeatureFlag>, FlagError> {
        #[cfg(feature = "cache")]
        if let Some(cache) = &self.cache {
            let key = cache_key(flag_name);
            if let Some(bytes) = cache.get(&key).await? {
                let cached: Option<FeatureFlag> = serde_json::from_slice(&bytes)?;
                return Ok(cached);
            }
        }

        let flag = FeatureFlag::objects()
            .filter(djangors_orm::q!(name = flag_name))?
            .all(&self.db)
            .await?
            .into_iter()
            .next();

        #[cfg(feature = "cache")]
        if let Some(cache) = &self.cache {
            let key = cache_key(flag_name);
            let bytes = serde_json::to_vec(&flag)?;
            let ttl = (self.cache_ttl_secs > 0).then(|| Duration::from_secs(self.cache_ttl_secs));
            cache.set(&key, bytes, ttl).await?;
        }

        Ok(flag)
    }

    #[cfg(feature = "cache")]
    async fn invalidate_cache(&self, flag_name: &str) -> Result<(), FlagError> {
        if let Some(cache) = &self.cache {
            cache.delete(&cache_key(flag_name)).await?;
        }
        Ok(())
    }

    #[cfg(not(feature = "cache"))]
    async fn invalidate_cache(&self, _flag_name: &str) -> Result<(), FlagError> {
        Ok(())
    }
}

#[cfg(feature = "cache")]
fn cache_key(flag_name: &str) -> String {
    format!("flags:{flag_name}")
}

/// Ensures the `djangors_feature_flag` table exists in the database.
pub async fn create_flags_table(db: &Database) -> Result<(), FlagError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let ts = dialect.timestamp_type();
    let bool_t = dialect.bool_type();

    let sql = format!(
        "CREATE TABLE IF NOT EXISTS djangors_feature_flag (
            id {pk},
            name VARCHAR(128) NOT NULL UNIQUE,
            description TEXT NOT NULL,
            enabled {bool_t} NOT NULL,
            rollout_percentage INTEGER NOT NULL,
            allowed_users TEXT NOT NULL,
            allowed_orgs TEXT NOT NULL,
            updated_at {ts} NOT NULL
        )"
    );

    conn.execute(&sql, &[])
        .await
        .map_err(djangors_db::DbError::from)?;
    Ok(())
}

/// Registers [`FeatureFlag`] with [`djangors_admin::AdminSite`].
#[cfg(feature = "admin")]
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<FeatureFlag>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use djangors_db::DatabaseConfig;

    async fn setup_db() -> Database {
        let config = DatabaseConfig::new("sqlite::memory:")
            .max_connections(1)
            .min_connections(1);
        let db = Database::connect(&config).await.unwrap();
        create_flags_table(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn test_crud_create_upsert_list_delete() {
        let db = setup_db().await;
        let svc = FlagService::new(db);

        // Create
        let created = svc
            .set_flag("new_ui", "Enable new UI", true, 50, &["u1"], &["o1"])
            .await
            .unwrap();
        assert_eq!(created.name, "new_ui");
        assert_eq!(created.description, "Enable new UI");
        assert!(created.enabled);
        assert_eq!(created.rollout_percentage, 50);

        let flags = svc.list_flags().await.unwrap();
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].id, created.id);

        // Upsert (update existing)
        let updated = svc
            .set_flag("new_ui", "Updated UI", false, 100, &["u2"], &[])
            .await
            .unwrap();
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.description, "Updated UI");
        assert!(!updated.enabled);
        assert_eq!(updated.rollout_percentage, 100);
        assert_eq!(svc.list_flags().await.unwrap().len(), 1);

        // Delete
        assert!(svc.delete_flag("new_ui").await.unwrap());
        assert!(!svc.delete_flag("new_ui").await.unwrap());
        assert!(svc.list_flags().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_global_toggle_and_missing_flag() {
        let db = setup_db().await;
        let svc = FlagService::new(db);
        let ctx = FlagContext::new().user("user-1").org("org-1");

        // Missing flag returns false
        assert!(!svc.is_enabled("missing", &ctx).await.unwrap());

        // Disabled flag overrides 100% rollout and allowlists
        svc.set_flag("beta", "Beta", false, 100, &["user-1"], &["org-1"])
            .await
            .unwrap();
        assert!(!svc.is_enabled("beta", &ctx).await.unwrap());

        // Enabled flag with 100% rollout returns true
        svc.set_flag("beta", "Beta", true, 100, &[], &[])
            .await
            .unwrap();
        assert!(svc.is_enabled("beta", &ctx).await.unwrap());
    }

    #[test]
    fn test_allowlisted_user_and_org_override() {
        let flag = FeatureFlag {
            id: 1,
            name: "vip_feature".to_string(),
            description: "VIP only".to_string(),
            enabled: true,
            rollout_percentage: 0,
            allowed_users: r#"["alice","bob"]"#.to_string(),
            allowed_orgs: r#"["acme"]"#.to_string(),
            updated_at: Utc::now(),
        };

        assert!(FlagService::evaluate(
            &flag,
            &FlagContext::new().user("alice")
        ));
        assert!(FlagService::evaluate(&flag, &FlagContext::new().org("acme")));
        assert!(!FlagService::evaluate(
            &flag,
            &FlagContext::new().user("charlie").org("other")
        ));
        assert!(!FlagService::evaluate(&flag, &FlagContext::new()));
    }

    #[test]
    fn test_deterministic_percentage_bucketing() {
        let flag = FeatureFlag {
            id: 1,
            name: "gradual_rollout".to_string(),
            description: "Gradual".to_string(),
            enabled: true,
            rollout_percentage: 30,
            allowed_users: "[]".to_string(),
            allowed_orgs: "[]".to_string(),
            updated_at: Utc::now(),
        };

        // Anonymous context without user or org gets false for partial rollouts
        assert!(!FlagService::evaluate(&flag, &FlagContext::new()));

        // Deterministic for same subject
        let ctx = FlagContext::new().user("user-42");
        let first = FlagService::evaluate(&flag, &ctx);
        for _ in 0..10 {
            assert_eq!(FlagService::evaluate(&flag, &ctx), first);
        }

        // Roughly 30% across 1000 users
        let enabled_count = (0..1000)
            .filter(|i| {
                FlagService::evaluate(&flag, &FlagContext::new().user(format!("user-{i}")))
            })
            .count();
        assert!(
            (200..=400).contains(&enabled_count),
            "expected ~300 enabled out of 1000, got {enabled_count}"
        );
    }

    #[cfg(feature = "cache")]
    #[tokio::test]
    async fn test_cache_hit_and_invalidation() {
        use djangors_cache::{Cache, InMemoryCache};

        let db = setup_db().await;
        let cache = Arc::new(InMemoryCache::default());
        let svc = FlagService::new(db.clone()).with_cache(cache.clone(), 60);
        let ctx = FlagContext::new().user("u1");

        svc.set_flag("cached_flag", "Cached", true, 100, &[], &[])
            .await
            .unwrap();

        // First call populates cache
        assert!(svc.is_enabled("cached_flag", &ctx).await.unwrap());
        let cached_bytes = cache.get("flags:cached_flag").await.unwrap();
        assert!(cached_bytes.is_some());

        // Mutate DB directly without going through FlagService -> cache still serves stale true
        let mut raw_flag = FeatureFlag::objects()
            .filter(djangors_orm::q!(name = "cached_flag"))
            .unwrap()
            .all(&db)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        raw_flag.enabled = false;
        raw_flag.update(&db).await.unwrap();
        assert!(svc.is_enabled("cached_flag", &ctx).await.unwrap());

        // set_flag invalidates cache and reflects updated state
        svc.set_flag("cached_flag", "Cached", false, 100, &[], &[])
            .await
            .unwrap();
        assert!(!svc.is_enabled("cached_flag", &ctx).await.unwrap());

        // Re-enable, populate cache, then delete_flag invalidates cache
        svc.set_flag("cached_flag", "Cached", true, 100, &[], &[])
            .await
            .unwrap();
        assert!(svc.is_enabled("cached_flag", &ctx).await.unwrap());
        assert!(svc.delete_flag("cached_flag").await.unwrap());
        assert!(cache.get("flags:cached_flag").await.unwrap().is_none());
        assert!(!svc.is_enabled("cached_flag", &ctx).await.unwrap());
    }
}
