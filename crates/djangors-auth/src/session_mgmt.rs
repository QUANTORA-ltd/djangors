//! Session management helpers for authentication: "Remember me" login,
//! server-side session listing/revocation, and session-generation invalidation.

use crate::{
    default_validators, hash_password, validate_password, AuthError, AuthUser, UserAttributes,
    SESSION_USER_ID_KEY,
};
use djangors_core::settings::DjangorsSettings;
use djangors_core::Request;
use djangors_db::{BindValue, Database};
use djangors_sessions::{hash_session_key, Session, SessionRecord, SessionStore};
use std::sync::Arc;
use std::time::Duration;

/// Session key storing the user's session generation counter for signed-cookie invalidation.
pub const SESSION_GENERATION_KEY: &str = "_auth_session_gen";

/// Default "remember me" lifetime (30 days) used when settings are not in request state.
const DEFAULT_REMEMBER_AGE_SECS: u64 = 30 * 24 * 60 * 60;

/// Options controlling login session behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginOptions {
    /// When `true`, issues a persistent session cookie with `Max-Age = settings.session_remember_age`
    /// (default 30 days). When `false`, issues a browser-session cookie without `Max-Age`.
    pub remember: bool,
}

impl LoginOptions {
    /// Creates `LoginOptions` with `remember` set.
    pub fn new(remember: bool) -> Self {
        Self { remember }
    }
}

async fn ensure_generation_table(db: &Database) -> Result<(), AuthError> {
    let mut conn = db.conn();
    let bigint = conn.dialect().bigint_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS auth_user_session_generation (
            user_id {bigint} PRIMARY KEY,
            generation {bigint} NOT NULL DEFAULT 0
        )"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(|e| AuthError::Session(e.to_string()))?;
    Ok(())
}

/// Returns the current session generation counter for `user_id` (0 if never bumped).
///
/// This runs on every authenticated request (via [`crate::Auth`]), so it issues a
/// single `SELECT` and never DDL. The table is created lazily by
/// [`bump_session_generation`]; until then no generation has ever been bumped,
/// so a missing table reads as 0.
pub async fn get_session_generation(db: &Database, user_id: i64) -> Result<i64, AuthError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let sql = format!("SELECT generation FROM auth_user_session_generation WHERE user_id = {p1}");
    match conn.fetch_optional(&sql, &[BindValue::I64(user_id)]).await {
        Ok(row) => Ok(row.and_then(|r| r.try_i64(0).ok().flatten()).unwrap_or(0)),
        Err(err) => {
            let msg = err.to_string().to_ascii_lowercase();
            if msg.contains("no such table") || msg.contains("does not exist") {
                Ok(0)
            } else {
                Err(AuthError::Session(err.to_string()))
            }
        }
    }
}

/// Increments and returns the new session generation counter for `user_id`.
/// Any existing signed-cookie session carrying an older generation is rejected by [`crate::Auth`].
pub async fn bump_session_generation(db: &Database, user_id: i64) -> Result<i64, AuthError> {
    ensure_generation_table(db).await?;
    let next = get_session_generation(db, user_id).await? + 1;
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let p1 = dialect.placeholder(1);
    let p2 = dialect.placeholder(2);
    let sql = format!(
        "INSERT INTO auth_user_session_generation (user_id, generation) VALUES ({p1}, {p2})
         ON CONFLICT (user_id) DO UPDATE SET generation = EXCLUDED.generation"
    );
    conn.execute(&sql, &[BindValue::I64(user_id), BindValue::I64(next)])
        .await
        .map_err(|e| AuthError::Session(e.to_string()))?;
    Ok(next)
}

/// Establishes an authenticated session on `session` with explicit `LoginOptions` and `remember_age`.
pub fn login_session_with<U: AuthUser>(
    session: &Session,
    user: &U,
    options: LoginOptions,
    remember_age: Duration,
) {
    session.cycle_key();
    session.set(SESSION_USER_ID_KEY, user.id());
    if options.remember {
        session.set_expiry(Some(remember_age));
    } else {
        session.set_expiry(None);
    }
}

/// Establishes an authenticated session for `user` on `req` using [`LoginOptions`].
///
/// - When `options.remember` is `false`, the session cookie has no `Max-Age` (browser session).
/// - When `options.remember` is `true`, `Max-Age` is set to `settings.session_remember_age`
///   (default 30 days).
/// - If a [`Database`] is present in request state, the user's current `session_generation`
///   is recorded in the session so "log out everywhere" works in signed-cookie mode too.
pub async fn login_with<U: AuthUser>(
    req: &Request,
    user: &U,
    options: LoginOptions,
) -> Result<(), AuthError> {
    let session = req
        .ext::<Session>()
        .ok_or_else(|| AuthError::Session("session extension absent".to_string()))?;
    let remember_secs = req
        .state::<DjangorsSettings>()
        .map(|s| s.session_remember_age)
        .unwrap_or(DEFAULT_REMEMBER_AGE_SECS);

    login_session_with(session, user, options, Duration::from_secs(remember_secs));

    if let Some(db) = req.state::<Database>() {
        let gen = get_session_generation(db, user.id()).await?;
        session.set(SESSION_GENERATION_KEY, gen);
    }
    Ok(())
}

/// Lists all active server-side sessions belonging to `user`.
pub async fn list_sessions<U: AuthUser>(
    store: &dyn SessionStore,
    user: &U,
) -> Result<Vec<SessionRecord>, AuthError> {
    store
        .for_user(user.id())
        .await
        .map_err(|e| AuthError::Session(e.to_string()))
}

/// Revokes a specific session belonging to `user` by its raw key or `key_hash`.
pub async fn revoke_session<U: AuthUser>(
    store: &dyn SessionStore,
    user: &U,
    key_or_hash: &str,
) -> Result<(), AuthError> {
    let target_hash = hash_session_key(key_or_hash);
    let owned = store
        .for_user(user.id())
        .await
        .map_err(|e| AuthError::Session(e.to_string()))?;
    let is_owned = owned
        .iter()
        .any(|rec| rec.key_hash == key_or_hash || rec.key_hash == target_hash);
    if is_owned {
        store
            .delete(key_or_hash)
            .await
            .map_err(|e| AuthError::Session(e.to_string()))?;
    }
    Ok(())
}

/// Logs out all other sessions for the authenticated user on `req`, keeping the current
/// session active (with a rotated key).
///
/// Works for both server-side [`SessionStore`]s (deletes other session rows) and
/// signed-cookie mode (increments the user's `session_generation` counter).
pub async fn logout_other_sessions(req: &Request) -> Result<(), AuthError> {
    let session = req
        .ext::<Session>()
        .ok_or_else(|| AuthError::Session("session extension absent".to_string()))?;
    let user_id = session
        .get::<i64>(SESSION_USER_ID_KEY)
        .ok_or_else(|| AuthError::Session("not authenticated".to_string()))?;

    let current_key = session.session_key();
    let current_hash = hash_session_key(&current_key);

    if let Some(store) = req.ext::<Arc<dyn SessionStore>>() {
        let sessions = store
            .for_user(user_id)
            .await
            .map_err(|e| AuthError::Session(e.to_string()))?;
        for rec in sessions {
            if rec.key_hash != current_hash {
                store
                    .delete(&rec.key_hash)
                    .await
                    .map_err(|e| AuthError::Session(e.to_string()))?;
            }
        }
    }

    session.cycle_key();

    if let Some(db) = req.state::<Database>() {
        let new_gen = bump_session_generation(db, user_id).await?;
        session.set(SESSION_GENERATION_KEY, new_gen);
    }

    Ok(())
}

/// Changes `user`'s password, rotates the current session key, and terminates all other
/// sessions for the user.
pub async fn change_password<U: AuthUser>(
    req: &Request,
    user: &mut U,
    new_password: &str,
) -> Result<(), AuthError> {
    let db = req
        .state::<Database>()
        .ok_or_else(|| AuthError::Session("database state absent".to_string()))?;

    let attrs = UserAttributes::new(user.username(), "");
    if let Err(errors) = validate_password(new_password, Some(&attrs), &default_validators()) {
        return Err(AuthError::WeakPassword(errors));
    }

    let hashed = hash_password(new_password)?;
    user.set_password_hash(hashed);
    user.update_user(db).await?;

    if req.ext::<Session>().is_some() {
        logout_other_sessions(req).await?;
    } else {
        bump_session_generation(db, user.id()).await?;
    }

    Ok(())
}
