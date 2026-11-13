//! Server-side session stores (`SessionStore`, `DatabaseStore`, `RedisStore`).

use crate::{generate_session_key, Session, SessionError, SessionExpiry, SessionInner};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use djangors_db::{BindValue, Database, NullKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Metadata about an active server-side session returned by [`SessionStore::for_user`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Hashed session key stored on the server.
    pub key_hash: String,
    /// Associated user ID, if the session is authenticated.
    pub user_id: Option<i64>,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
    /// Client IP address recorded for this session, if available.
    pub ip: Option<String>,
    /// Client `User-Agent` header recorded for this session, if available.
    pub user_agent: Option<String>,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Last-seen timestamp (UTC), updated at most once per minute.
    pub last_seen_at: DateTime<Utc>,
}

/// Request context passed to [`SessionStore::save`] for audit metadata.
#[derive(Debug, Clone, Default)]
pub struct SessionContext {
    /// Client IP address extracted from request headers, if present.
    pub ip: Option<String>,
    /// Client `User-Agent` header value, if present.
    pub user_agent: Option<String>,
}

/// Hashes a raw session key with SHA-256 and returns lowercase hex.
pub fn hash_session_key(raw_key: &str) -> String {
    let digest = Sha256::digest(raw_key.as_bytes());
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Session data as written to a server-side store: everything except the raw
/// `_session_key`. Only its SHA-256 hash is stored server-side, so a leaked
/// sessions table or Redis dump can't be replayed as live cookies.
fn persisted_data(data: &HashMap<String, serde_json::Value>) -> HashMap<String, serde_json::Value> {
    let mut out = data.clone();
    out.remove("_session_key");
    out
}

/// Puts the raw key back into loaded session data so [`Session::session_key`] works.
fn restore_session_key(data: &mut HashMap<String, serde_json::Value>, raw_key: &str) {
    data.insert(
        "_session_key".to_string(),
        serde_json::Value::String(raw_key.to_string()),
    );
}

/// Pluggable backend for loading, saving, deleting, and listing sessions.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
    /// Cookie name used by this store.
    fn cookie_name(&self) -> &str;

    /// Default session lifetime.
    fn max_age(&self) -> Duration;

    /// Whether the `Secure` attribute should be set on session cookies.
    fn secure(&self) -> bool;

    /// Loads a session from the given cookie value.
    /// Returns `Ok(None)` if the session does not exist, is invalid, or has expired.
    async fn load(&self, cookie_value: &str) -> Result<Option<Session>, SessionError>;

    /// Saves the session and returns the cookie value to send to the client.
    async fn save(&self, session: &Session, ctx: &SessionContext) -> Result<String, SessionError>;

    /// Deletes a session by raw session key or hashed key.
    async fn delete(&self, key_or_hash: &str) -> Result<(), SessionError>;

    /// Lists all active (non-expired) sessions belonging to `user_id`.
    async fn for_user(&self, user_id: i64) -> Result<Vec<SessionRecord>, SessionError>;
}

/// Database-backed session store using the `djangors_session` table.
///
/// Session keys are 32 random bytes (64 hex chars) sent to the client and
/// stored as SHA-256 hashes in the database. `last_seen_at` is throttled
/// to at most one write per minute.
#[derive(Clone)]
pub struct DatabaseStore {
    db: Database,
    cookie_name: String,
    max_age: Duration,
    secure: bool,
}

impl DatabaseStore {
    /// Creates a new `DatabaseStore` backed by `db`.
    pub fn new(db: Database) -> Self {
        Self {
            db,
            cookie_name: "djangors_sessionid".to_string(),
            max_age: Duration::from_secs(14 * 24 * 60 * 60),
            secure: false,
        }
    }

    /// Configures a custom cookie name.
    pub fn with_cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }

    /// Configures the default session lifetime.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Configures whether the `Secure` cookie flag should be set.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    /// Ensures the `djangors_session` table and index exist.
    pub async fn ensure_table(&self) -> Result<(), SessionError> {
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let ts = dialect.timestamp_type();
        let bigint = dialect.bigint_type();
        let create_sql = format!(
            "CREATE TABLE IF NOT EXISTS djangors_session (
                key TEXT PRIMARY KEY,
                user_id {bigint},
                data TEXT NOT NULL,
                expires_at {ts} NOT NULL,
                ip TEXT,
                user_agent TEXT,
                created_at {ts} NOT NULL,
                last_seen_at {ts} NOT NULL
            )"
        );
        conn.execute(&create_sql, &[])
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        let idx_sql =
            "CREATE INDEX IF NOT EXISTS idx_djangors_session_user_id ON djangors_session (user_id)";
        conn.execute(idx_sql, &[])
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;
        Ok(())
    }

    /// Deletes all expired session rows from `djangors_session` and returns the count removed.
    pub async fn clear_expired(&self) -> Result<u64, SessionError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql = format!("DELETE FROM djangors_session WHERE expires_at <= {p1}");
        let affected = conn
            .execute(&sql, &[BindValue::DateTime(Utc::now())])
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;
        Ok(affected)
    }

    fn compute_expiry(&self, mode: SessionExpiry) -> DateTime<Utc> {
        let dur = match mode {
            SessionExpiry::Default | SessionExpiry::BrowserSession => self.max_age,
            SessionExpiry::MaxAge(d) => d,
        };
        let chrono_dur = chrono::Duration::from_std(dur).unwrap_or_default();
        Utc::now() + chrono_dur
    }
}

#[async_trait]
impl SessionStore for DatabaseStore {
    fn cookie_name(&self) -> &str {
        &self.cookie_name
    }

    fn max_age(&self) -> Duration {
        self.max_age
    }

    fn secure(&self) -> bool {
        self.secure
    }

    async fn load(&self, cookie_value: &str) -> Result<Option<Session>, SessionError> {
        self.ensure_table().await?;
        let key_hash = hash_session_key(cookie_value);
        let mut conn = self.db.conn();
        let p1 = conn.dialect().placeholder(1);
        let sql =
            format!("SELECT data, expires_at, last_seen_at FROM djangors_session WHERE key = {p1}");
        let row_opt = conn
            .fetch_optional(&sql, &[BindValue::Text(key_hash.clone())])
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        let Some(row) = row_opt else {
            return Ok(None);
        };

        let data_json = row
            .try_string(0)
            .map_err(|e| SessionError::Backend(e.to_string()))?
            .unwrap_or_else(|| "{}".to_string());
        let expires_at = row
            .try_datetime(1)
            .map_err(|e| SessionError::Backend(e.to_string()))?;
        let last_seen_at = row
            .try_datetime(2)
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        let now = Utc::now();
        if expires_at.is_some_and(|exp| exp <= now) {
            let del_sql = format!("DELETE FROM djangors_session WHERE key = {p1}");
            let _ = conn.execute(&del_sql, &[BindValue::Text(key_hash)]).await;
            return Ok(None);
        }

        if let Some(last_seen) = last_seen_at {
            if (now - last_seen) >= chrono::Duration::seconds(60) {
                let p2 = conn.dialect().placeholder(2);
                let upd_sql =
                    format!("UPDATE djangors_session SET last_seen_at = {p1} WHERE key = {p2}");
                let _ = conn
                    .execute(
                        &upd_sql,
                        &[BindValue::DateTime(now), BindValue::Text(key_hash)],
                    )
                    .await;
            }
        }

        let mut data: HashMap<String, serde_json::Value> = serde_json::from_str(&data_json)
            .map_err(|e| SessionError::Serialization(e.to_string()))?;
        restore_session_key(&mut data, cookie_value);
        let expiry_mode = crate::expiry_from_data(&data);

        Ok(Some(Session {
            inner: Arc::new(Mutex::new(SessionInner {
                data,
                modified: false,
                cycled: false,
                new: false,
                expiry: expiry_mode,
                loaded_key: Some(cookie_value.to_string()),
            })),
        }))
    }

    async fn save(&self, session: &Session, ctx: &SessionContext) -> Result<String, SessionError> {
        self.ensure_table().await?;
        let (raw_key, old_key_to_delete, data_json, expires_at, user_id) = {
            let mut inner = session.inner.lock().unwrap();
            let mut old_to_delete = None;
            let raw_key = if inner.cycled || inner.loaded_key.is_none() {
                if inner.cycled {
                    if let Some(prev) = inner.loaded_key.take() {
                        old_to_delete = Some(hash_session_key(&prev));
                    }
                }
                let fresh = generate_session_key();
                inner.data.insert(
                    "_session_key".to_string(),
                    serde_json::Value::String(fresh.clone()),
                );
                inner.cycled = false;
                inner.loaded_key = Some(fresh.clone());
                fresh
            } else {
                inner.loaded_key.clone().unwrap()
            };

            let user_id = inner.data.get("_auth_user_id").and_then(|v| v.as_i64());
            let expires_at = self.compute_expiry(inner.expiry);
            let data_json = serde_json::to_string(&persisted_data(&inner.data))
                .map_err(|e| SessionError::Serialization(e.to_string()))?;
            (raw_key, old_to_delete, data_json, expires_at, user_id)
        };

        let mut conn = self.db.conn();
        if let Some(old_hash) = old_key_to_delete {
            let p1 = conn.dialect().placeholder(1);
            let del_sql = format!("DELETE FROM djangors_session WHERE key = {p1}");
            let _ = conn.execute(&del_sql, &[BindValue::Text(old_hash)]).await;
        }

        let key_hash = hash_session_key(&raw_key);
        let now = Utc::now();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let p2 = dialect.placeholder(2);
        let p3 = dialect.placeholder(3);
        let p4 = dialect.placeholder(4);
        let p5 = dialect.placeholder(5);
        let p6 = dialect.placeholder(6);
        let p7 = dialect.placeholder(7);
        let p8 = dialect.placeholder(8);

        let sql = format!(
            "INSERT INTO djangors_session (key, user_id, data, expires_at, ip, user_agent, created_at, last_seen_at)
             VALUES ({p1}, {p2}, {p3}, {p4}, {p5}, {p6}, {p7}, {p8})
             ON CONFLICT (key) DO UPDATE SET
                user_id = EXCLUDED.user_id,
                data = EXCLUDED.data,
                expires_at = EXCLUDED.expires_at,
                ip = COALESCE(EXCLUDED.ip, djangors_session.ip),
                user_agent = COALESCE(EXCLUDED.user_agent, djangors_session.user_agent),
                last_seen_at = EXCLUDED.last_seen_at"
        );

        let params = vec![
            BindValue::Text(key_hash),
            match user_id {
                Some(id) => BindValue::I64(id),
                None => BindValue::Null(NullKind::I64),
            },
            BindValue::Text(data_json),
            BindValue::DateTime(expires_at),
            match &ctx.ip {
                Some(ip) => BindValue::Text(ip.clone()),
                None => BindValue::Null(NullKind::Text),
            },
            match &ctx.user_agent {
                Some(ua) => BindValue::Text(ua.clone()),
                None => BindValue::Null(NullKind::Text),
            },
            BindValue::DateTime(now),
            BindValue::DateTime(now),
        ];

        conn.execute(&sql, &params)
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        Ok(raw_key)
    }

    async fn delete(&self, key_or_hash: &str) -> Result<(), SessionError> {
        self.ensure_table().await?;
        let hashed = hash_session_key(key_or_hash);
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let p2 = dialect.placeholder(2);
        let sql = format!("DELETE FROM djangors_session WHERE key = {p1} OR key = {p2}");
        conn.execute(
            &sql,
            &[
                BindValue::Text(hashed),
                BindValue::Text(key_or_hash.to_string()),
            ],
        )
        .await
        .map_err(|e| SessionError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn for_user(&self, user_id: i64) -> Result<Vec<SessionRecord>, SessionError> {
        self.ensure_table().await?;
        let mut conn = self.db.conn();
        let dialect = conn.dialect();
        let p1 = dialect.placeholder(1);
        let p2 = dialect.placeholder(2);
        let sql = format!(
            "SELECT key, user_id, expires_at, ip, user_agent, created_at, last_seen_at
             FROM djangors_session
             WHERE user_id = {p1} AND expires_at > {p2}
             ORDER BY last_seen_at DESC"
        );
        let rows = conn
            .fetch_all(
                &sql,
                &[BindValue::I64(user_id), BindValue::DateTime(Utc::now())],
            )
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            let key_hash = row
                .try_string(0)
                .map_err(|e| SessionError::Backend(e.to_string()))?
                .unwrap_or_default();
            let uid = row
                .try_i64(1)
                .map_err(|e| SessionError::Backend(e.to_string()))?;
            let expires_at = row
                .try_datetime(2)
                .map_err(|e| SessionError::Backend(e.to_string()))?
                .unwrap_or_else(Utc::now);
            let ip = row
                .try_string(3)
                .map_err(|e| SessionError::Backend(e.to_string()))?;
            let user_agent = row
                .try_string(4)
                .map_err(|e| SessionError::Backend(e.to_string()))?;
            let created_at = row
                .try_datetime(5)
                .map_err(|e| SessionError::Backend(e.to_string()))?
                .unwrap_or_else(Utc::now);
            let last_seen_at = row
                .try_datetime(6)
                .map_err(|e| SessionError::Backend(e.to_string()))?
                .unwrap_or_else(Utc::now);
            records.push(SessionRecord {
                key_hash,
                user_id: uid,
                expires_at,
                ip,
                user_agent,
                created_at,
                last_seen_at,
            });
        }
        Ok(records)
    }
}

/// Redis-backed session store (enabled via the `redis` feature).
#[cfg(feature = "redis")]
#[derive(Clone)]
pub struct RedisStore {
    client: redis::Client,
    key_prefix: String,
    cookie_name: String,
    max_age: Duration,
    secure: bool,
}

#[cfg(feature = "redis")]
#[derive(Serialize, Deserialize)]
struct RedisSessionPayload {
    data: HashMap<String, serde_json::Value>,
    user_id: Option<i64>,
    expires_at: DateTime<Utc>,
    ip: Option<String>,
    user_agent: Option<String>,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
}

#[cfg(feature = "redis")]
impl RedisStore {
    /// Creates a new `RedisStore` connecting to `url`.
    pub fn new(url: &str) -> Result<Self, SessionError> {
        let client = redis::Client::open(url).map_err(|e| SessionError::Backend(e.to_string()))?;
        Ok(Self {
            client,
            key_prefix: "djangors:session:".to_string(),
            cookie_name: "djangors_sessionid".to_string(),
            max_age: Duration::from_secs(14 * 24 * 60 * 60),
            secure: false,
        })
    }

    /// Configures a custom Redis key prefix.
    pub fn with_key_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Configures a custom cookie name.
    pub fn with_cookie_name(mut self, name: impl Into<String>) -> Self {
        self.cookie_name = name.into();
        self
    }

    /// Configures the default session lifetime.
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Configures whether the `Secure` cookie flag should be set.
    pub fn with_secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    async fn connection(&self) -> Result<redis::aio::MultiplexedConnection, SessionError> {
        self.client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))
    }

    fn session_redis_key(&self, key_hash: &str) -> String {
        format!("{}{}", self.key_prefix, key_hash)
    }

    fn user_set_key(&self, user_id: i64) -> String {
        format!("{}user:{}", self.key_prefix, user_id)
    }
}

#[cfg(feature = "redis")]
#[async_trait]
impl SessionStore for RedisStore {
    fn cookie_name(&self) -> &str {
        &self.cookie_name
    }

    fn max_age(&self) -> Duration {
        self.max_age
    }

    fn secure(&self) -> bool {
        self.secure
    }

    async fn load(&self, cookie_value: &str) -> Result<Option<Session>, SessionError> {
        let key_hash = hash_session_key(cookie_value);
        let redis_key = self.session_redis_key(&key_hash);
        let mut conn = self.connection().await?;
        let raw: Option<String> = redis::cmd("GET")
            .arg(&redis_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        let Some(json) = raw else {
            return Ok(None);
        };
        let mut payload: RedisSessionPayload =
            serde_json::from_str(&json).map_err(|e| SessionError::Serialization(e.to_string()))?;

        let now = Utc::now();
        if payload.expires_at <= now {
            let _: () = redis::cmd("DEL")
                .arg(&redis_key)
                .query_async(&mut conn)
                .await
                .map_err(|e| SessionError::Backend(e.to_string()))?;
            return Ok(None);
        }

        if (now - payload.last_seen_at) >= chrono::Duration::seconds(60) {
            payload.last_seen_at = now;
            let remaining_ms = (payload.expires_at - now).num_milliseconds().max(1) as u64;
            if let Ok(updated) = serde_json::to_string(&payload) {
                let _: Result<(), _> = redis::cmd("SET")
                    .arg(&redis_key)
                    .arg(updated)
                    .arg("PX")
                    .arg(remaining_ms)
                    .query_async(&mut conn)
                    .await;
            }
        }

        restore_session_key(&mut payload.data, cookie_value);
        let expiry_mode = crate::expiry_from_data(&payload.data);
        Ok(Some(Session {
            inner: Arc::new(Mutex::new(SessionInner {
                data: payload.data,
                modified: false,
                cycled: false,
                new: false,
                expiry: expiry_mode,
                loaded_key: Some(cookie_value.to_string()),
            })),
        }))
    }

    async fn save(&self, session: &Session, ctx: &SessionContext) -> Result<String, SessionError> {
        let (raw_key, old_hash, payload, ttl_ms, user_id) = {
            let mut inner = session.inner.lock().unwrap();
            let mut old_hash = None;
            let raw_key = if inner.cycled || inner.loaded_key.is_none() {
                if inner.cycled {
                    if let Some(prev) = inner.loaded_key.take() {
                        old_hash = Some(hash_session_key(&prev));
                    }
                }
                let fresh = generate_session_key();
                inner.data.insert(
                    "_session_key".to_string(),
                    serde_json::Value::String(fresh.clone()),
                );
                inner.cycled = false;
                inner.loaded_key = Some(fresh.clone());
                fresh
            } else {
                inner.loaded_key.clone().unwrap()
            };

            let dur = match inner.expiry {
                SessionExpiry::Default | SessionExpiry::BrowserSession => self.max_age,
                SessionExpiry::MaxAge(d) => d,
            };
            let now = Utc::now();
            let expires_at = now + chrono::Duration::from_std(dur).unwrap_or_default();
            let user_id = inner.data.get("_auth_user_id").and_then(|v| v.as_i64());
            let payload = RedisSessionPayload {
                data: persisted_data(&inner.data),
                user_id,
                expires_at,
                ip: ctx.ip.clone(),
                user_agent: ctx.user_agent.clone(),
                created_at: now,
                last_seen_at: now,
            };
            let ttl_ms = dur.as_millis().max(1) as u64;
            (raw_key, old_hash, payload, ttl_ms, user_id)
        };

        let mut conn = self.connection().await?;
        if let Some(prev_hash) = old_hash {
            let _: Result<(), _> = redis::cmd("DEL")
                .arg(self.session_redis_key(&prev_hash))
                .query_async(&mut conn)
                .await;
        }

        let key_hash = hash_session_key(&raw_key);
        let json = serde_json::to_string(&payload)
            .map_err(|e| SessionError::Serialization(e.to_string()))?;

        redis::cmd("SET")
            .arg(self.session_redis_key(&key_hash))
            .arg(json)
            .arg("PX")
            .arg(ttl_ms)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        if let Some(uid) = user_id {
            redis::cmd("SADD")
                .arg(self.user_set_key(uid))
                .arg(&key_hash)
                .query_async::<()>(&mut conn)
                .await
                .map_err(|e| SessionError::Backend(e.to_string()))?;
        }

        Ok(raw_key)
    }

    async fn delete(&self, key_or_hash: &str) -> Result<(), SessionError> {
        let mut conn = self.connection().await?;
        let hashed = hash_session_key(key_or_hash);
        for candidate in [hashed, key_or_hash.to_string()] {
            let redis_key = self.session_redis_key(&candidate);
            let raw: Option<String> = redis::cmd("GET")
                .arg(&redis_key)
                .query_async(&mut conn)
                .await
                .map_err(|e| SessionError::Backend(e.to_string()))?;
            if let Some(json) = raw {
                if let Ok(payload) = serde_json::from_str::<RedisSessionPayload>(&json) {
                    if let Some(uid) = payload.user_id {
                        let _: Result<(), _> = redis::cmd("SREM")
                            .arg(self.user_set_key(uid))
                            .arg(&candidate)
                            .query_async(&mut conn)
                            .await;
                    }
                }
            }
            let _: () = redis::cmd("DEL")
                .arg(&redis_key)
                .query_async(&mut conn)
                .await
                .map_err(|e| SessionError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    async fn for_user(&self, user_id: i64) -> Result<Vec<SessionRecord>, SessionError> {
        let mut conn = self.connection().await?;
        let set_key = self.user_set_key(user_id);
        let members: Vec<String> = redis::cmd("SMEMBERS")
            .arg(&set_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| SessionError::Backend(e.to_string()))?;

        let now = Utc::now();
        let mut records = Vec::new();
        for key_hash in members {
            let raw: Option<String> = redis::cmd("GET")
                .arg(self.session_redis_key(&key_hash))
                .query_async(&mut conn)
                .await
                .map_err(|e| SessionError::Backend(e.to_string()))?;
            let Some(json) = raw else {
                let _: Result<(), _> = redis::cmd("SREM")
                    .arg(&set_key)
                    .arg(&key_hash)
                    .query_async(&mut conn)
                    .await;
                continue;
            };
            if let Ok(payload) = serde_json::from_str::<RedisSessionPayload>(&json) {
                if payload.expires_at > now {
                    records.push(SessionRecord {
                        key_hash,
                        user_id: payload.user_id,
                        expires_at: payload.expires_at,
                        ip: payload.ip,
                        user_agent: payload.user_agent,
                        created_at: payload.created_at,
                        last_seen_at: payload.last_seen_at,
                    });
                }
            }
        }
        records.sort_by_key(|r| std::cmp::Reverse(r.last_seen_at));
        Ok(records)
    }
}
