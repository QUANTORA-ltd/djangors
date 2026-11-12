#![deny(missing_docs)]
//! Authentication, users, and permissions for Djangors.
//!
//! Rust-idiomatic equivalent of Django's `AUTH_USER_MODEL` swap is compile-time genericity
//! over the `AuthUser` trait, not a settings string.

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use base64::Engine as _;
use djangors_core::signals::Signal;
use djangors_macros::Model;
use djangors_orm::ForeignKey;
use djangors_orm::Model as _;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

/// Email verification workflow (`EmailAddress`, signed tokens, `HasVerifiedEmail`).
pub mod email_verification;
/// GDPR user data export and scheduled account deletion / anonymization.
pub mod gdpr;
/// Admin impersonation (`start_impersonation_view`, `stop_impersonation_view`, `NotImpersonating`).
pub mod impersonation;
/// User invitation workflow (`Invitation`, `invite`, `accept_invitation`, `revoke_invitation`).
pub mod invitations;
/// LDAP / Active Directory authentication backend (`LdapBackend`, `LdapConfig`).
#[cfg(feature = "ldap")]
pub mod ldap;
/// Passwordless magic-link login (`send_login_link`, `consume_login_link`, `magic_link_router`).
pub mod magic_link;
#[cfg(feature = "passkeys")]
pub mod passkeys;
/// Password complexity validators (`MinimumLength`, `NumericPassword`, etc.).
pub mod password_validation;
/// Session management helpers ("remember me", session listing, revocation, generation invalidation).
pub mod session_mgmt;

pub use email_verification::{
    create_email_address_table, ensure_verified_for_login, generate_email_verification_token,
    has_verified_email, parse_email_verification_token, send_verification_email, verify_email,
    EmailAddress, HasVerifiedEmail,
};
pub use gdpr::{
    cancel_account_deletion, download_export_view, export_user_data, get_scheduled_deletion,
    request_account_deletion, request_export_view, run_scheduled_deletions, AnonymizeHook,
    UserDataRegistry,
};
pub use impersonation::{
    create_impersonation_log_table, ensure_not_impersonating, impersonation_banner_html,
    impersonation_router, is_impersonating, NotImpersonating, RequestImpersonation,
    IMPERSONATOR_KEY,
};
pub use invitations::{
    accept_invitation, accept_invitation_page, accept_invitation_submit, admin_invitations_list,
    create_invitation_table, invitation_router, invite, resend_invitation, revoke_invitation,
    Invitation,
};
#[cfg(feature = "ldap")]
pub use ldap::{
    escape_dn_value, escape_filter_value, LdapAttributeMap, LdapBackend, LdapConfig,
    LdapGroupMapping, LdapUserProfile,
};
pub use magic_link::{
    consume_login_link, create_login_link_tables, hash_login_token, magic_link_router,
    send_login_link, LoginLink, LoginLinkPolicy,
};
#[cfg(feature = "password-strength")]
pub use password_validation::StrengthValidator;
pub use password_validation::{
    default_validators, validate_password, MinimumLength, NumericPassword, PasswordValidator,
    UserAttributeSimilarity, UserAttributes,
};
pub use session_mgmt::{
    bump_session_generation, change_password, get_session_generation, list_sessions,
    login_session_with, login_with, logout_other_sessions, revoke_session, LoginOptions,
    SESSION_GENERATION_KEY,
};

type HmacSha256 = Hmac<Sha256>;

/// Error type for authentication and user operations.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum AuthError {
    /// Password hashing or verification failed.
    #[error("password hashing error: {0}")]
    Hashing(String),
    /// Database error during authentication or permission lookup.
    #[error("database error: {0}")]
    Database(#[from] djangors_orm::OrmError),
    /// Too many failed login attempts within the rate limit window.
    #[error("too many login attempts, try again later")]
    RateLimited,
    /// The account is locked out after too many failed attempts, persisted across
    /// process restarts. Distinct from [`AuthError::RateLimited`]: a lockout rejects
    /// *correct* credentials too until it expires, whereas rate limiting only
    /// throttles the rate of attempts.
    #[error(
        "account locked due to too many failed login attempts, try again in {retry_after_secs}s"
    )]
    AccountLocked {
        /// Seconds remaining until the lockout expires.
        retry_after_secs: u64,
    },
    /// Provided password reset or authentication token is invalid or expired.
    #[error("invalid or expired token")]
    InvalidToken,
    /// Mail sending failed during password reset workflow.
    #[error("mail sending error: {0}")]
    Mail(String),
    /// The supplied password did not satisfy password complexity validators.
    #[error("weak password: {}", .0.join("; "))]
    WeakPassword(Vec<String>),
    /// Session store or session state error.
    #[error("session error: {0}")]
    Session(String),
    /// User's email address is not verified.
    #[error("email address is not verified")]
    EmailNotVerified,
    /// Logged-in user's email does not match the invitation email.
    #[error("invitation belongs to a different email address")]
    InvitationEmailMismatch,
    /// GDPR export or deletion error.
    #[error("gdpr error: {0}")]
    Gdpr(String),
    /// LDAP / Active Directory communication or configuration error.
    #[error("ldap error: {0}")]
    Ldap(String),
}

impl From<AuthError> for djangors_core::DjangorsError {
    fn from(err: AuthError) -> Self {
        use djangors_core::StatusCode;
        match err {
            AuthError::RateLimited => djangors_core::DjangorsError::api(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                err.to_string(),
            ),
            AuthError::AccountLocked { retry_after_secs } => djangors_core::DjangorsError::api(
                StatusCode::TOO_MANY_REQUESTS,
                "account_locked",
                err.to_string(),
            )
            .with_details(serde_json::json!({ "retry_after_secs": retry_after_secs })),
            AuthError::InvalidToken => djangors_core::DjangorsError::api(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                err.to_string(),
            ),
            AuthError::Hashing(msg) => {
                djangors_core::DjangorsError::Internal(format!("password hashing error: {msg}"))
            }
            AuthError::Database(db_err) => {
                djangors_core::DjangorsError::Internal(db_err.to_string())
            }
            AuthError::Mail(msg) => {
                djangors_core::DjangorsError::Internal(format!("mail sending error: {msg}"))
            }
            AuthError::WeakPassword(errors) => djangors_core::DjangorsError::api(
                StatusCode::BAD_REQUEST,
                "weak_password",
                "Password does not meet complexity requirements",
            )
            .with_details(serde_json::json!({ "errors": errors })),
            AuthError::Session(msg) => {
                djangors_core::DjangorsError::Unauthorized(format!("session error: {msg}"))
            }
            AuthError::EmailNotVerified => djangors_core::DjangorsError::api(
                StatusCode::FORBIDDEN,
                "email_not_verified",
                err.to_string(),
            ),
            AuthError::InvitationEmailMismatch => djangors_core::DjangorsError::api(
                StatusCode::FORBIDDEN,
                "invitation_email_mismatch",
                err.to_string(),
            ),
            AuthError::Gdpr(msg) => djangors_core::DjangorsError::Internal(msg),
            AuthError::Ldap(msg) => {
                djangors_core::DjangorsError::Internal(format!("ldap error: {msg}"))
            }
        }
    }
}

const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$dGVzdHBhc3N3b3Jk";

/// Payload for the [`LOGIN_SUCCEEDED`] signal.
#[derive(Debug, Clone)]
pub struct LoginSucceeded {
    /// User ID of the user who logged in.
    pub user_id: i64,
    /// Username of the user who logged in.
    pub username: String,
}

/// Payload for the [`LOGIN_FAILED`] signal.
///
/// NOTE: The `username` string is attacker-controlled input. It must never be
/// used unescaped in any context where that could pose a security risk (e.g. html).
#[derive(Debug, Clone)]
pub struct LoginFailed {
    /// Username attempt submitted during failed login.
    pub username: String,
}

/// Payload for the [`LOGGED_OUT`] signal.
#[derive(Debug, Clone)]
pub struct LoggedOut {
    /// Optional user ID of the user who logged out.
    pub user_id: Option<i64>,
}

/// Signal emitted when a user successfully authenticates.
pub static LOGIN_SUCCEEDED: LazyLock<Signal<LoginSucceeded>> = LazyLock::new(Signal::new);
/// Signal emitted when an authentication attempt fails.
pub static LOGIN_FAILED: LazyLock<Signal<LoginFailed>> = LazyLock::new(Signal::new);
/// Signal emitted when a user logs out.
pub static LOGGED_OUT: LazyLock<Signal<LoggedOut>> = LazyLock::new(Signal::new);

/// Trait defining an authentication backend mechanism.
#[async_trait::async_trait]
pub trait AuthBackend {
    /// The user model type produced by this authentication backend.
    type User: AuthUser;

    /// Authenticates a user using `username` and `password`.
    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<Self::User>, AuthError>;
}

/// Default database model authentication backend checking username and password hash.
pub struct ModelBackend;

#[async_trait::async_trait]
impl AuthBackend for ModelBackend {
    type User = User;

    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<User>, AuthError> {
        let mut users = User::objects()
            .filter(djangors_orm::q!(username = username))?
            .all(db)
            .await?;

        let user_opt = if users.len() == 1 {
            Some(users.remove(0))
        } else {
            None
        };

        let hash_to_verify = match &user_opt {
            Some(u) => u.password_hash(),
            None => DUMMY_HASH,
        };

        let verified = verify_password(password, hash_to_verify)?;

        if let Some(user) = user_opt {
            if verified && user.is_active() {
                LOGIN_SUCCEEDED
                    .send(LoginSucceeded {
                        user_id: user.id(),
                        username: username.to_string(),
                    })
                    .await;
                return Ok(Some(user));
            }
        }

        LOGIN_FAILED
            .send(LoginFailed {
                username: username.to_string(),
            })
            .await;
        Ok(None)
    }
}

/// A single-process, in-memory sliding-window login rate limiter around an
/// inner [`AuthBackend`].
///
/// State lives in this process's memory. For multi-process deployments that
/// need shared lockout state across instances, wrap your backend in
/// [`PersistentLockoutBackend`] (which stores failed-attempt counters and
/// lockout timestamps in the database) or pair endpoints with
/// `djangors_core::ratelimit::RateLimiter` (backed by `djangors-cache`,
/// including `RedisCache` and `DatabaseCache`).
pub struct RateLimitedBackend<B: AuthBackend> {
    inner: B,
    limiter: Mutex<HashMap<String, Vec<Instant>>>,
    max_attempts: u32,
    window: Duration,
}

impl<B: AuthBackend> RateLimitedBackend<B> {
    /// Constructs a rate limited wrapper backend around `inner` with `max_attempts` per `window`.
    pub fn new(inner: B, max_attempts: u32, window: Duration) -> Self {
        Self {
            inner,
            limiter: Mutex::new(HashMap::new()),
            max_attempts,
            window,
        }
    }

    /// Constructs a rate limited wrapper backend with default throttling (5 attempts per 15 minutes).
    pub fn default_login_throttle(inner: B) -> Self {
        Self::new(inner, 5, Duration::from_secs(15 * 60))
    }
}

#[async_trait::async_trait]
impl<B: AuthBackend + Send + Sync> AuthBackend for RateLimitedBackend<B> {
    type User = B::User;

    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<Self::User>, AuthError> {
        let now = Instant::now();
        let limit_exceeded = {
            let mut limiter = self.limiter.lock().unwrap();
            let attempts = limiter.entry(username.to_string()).or_default();
            attempts.retain(|&t| now.duration_since(t) < self.window);

            if attempts.len() as u32 >= self.max_attempts {
                true
            } else {
                attempts.push(now);
                false
            }
        };

        if limit_exceeded {
            LOGIN_FAILED
                .send(LoginFailed {
                    username: username.to_string(),
                })
                .await;
            return Err(AuthError::RateLimited);
        }

        self.inner.authenticate(db, username, password).await
    }
}

/// Persisted per-username login-failure tracking, backing [`PersistentLockoutBackend`].
/// The `django-axes` equivalent: unlike [`RateLimitedBackend`] (in-memory, per-process,
/// throttles the *rate* of attempts), this survives process restarts and rejects
/// *correct* credentials too once `locked_until` is in the future.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_login_lockout")]
pub struct LoginLockout {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// The username this lockout state tracks.
    #[djangors(max_length = 150, unique)]
    pub username: String,
    /// Consecutive failed attempts since the last reset (a successful login, or
    /// the previous lockout window expiring).
    pub failed_attempts: i32,
    /// When the current failure streak started.
    pub first_failed_at: chrono::DateTime<chrono::Utc>,
    /// If set and in the future, further attempts (even with correct credentials)
    /// are rejected with [`AuthError::AccountLocked`] until this time passes.
    pub locked_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// A persistent, database-backed account lockout wrapper around an inner
/// [`AuthBackend`], mirroring `django-axes`. After `max_attempts` consecutive
/// failures for a username, the account is locked for `lockout_duration` -
/// during that window, even a request with the *correct* password is rejected
/// with [`AuthError::AccountLocked`], which [`RateLimitedBackend`] alone does not
/// do (it only throttles how often attempts are checked at all). State is stored
/// in the `auth_login_lockout` table via [`LoginLockout`], so it survives process
/// restarts and is shared correctly across multiple app instances pointed at the
/// same database - the gap [`RateLimitedBackend`]'s own docs call out as
/// deliberately out of scope for that simpler, in-memory limiter.
///
/// A successful login clears the failure streak. An expired lockout (`locked_until`
/// in the past) is treated as a fresh window: the next failure starts counting from
/// 1, not from wherever the previous streak left off.
pub struct PersistentLockoutBackend<B: AuthBackend> {
    inner: B,
    max_attempts: u32,
    lockout_duration: chrono::Duration,
}

impl<B: AuthBackend> PersistentLockoutBackend<B> {
    /// Constructs a persistent lockout wrapper: `max_attempts` consecutive failures
    /// locks the account for `lockout_duration`.
    pub fn new(inner: B, max_attempts: u32, lockout_duration: std::time::Duration) -> Self {
        Self {
            inner,
            max_attempts,
            lockout_duration: chrono::Duration::from_std(lockout_duration)
                .unwrap_or(chrono::Duration::hours(1)),
        }
    }

    /// Constructs a persistent lockout wrapper with `django-axes`'s own common
    /// default: 5 attempts, 1 hour lockout.
    pub fn default_lockout(inner: B) -> Self {
        Self::new(inner, 5, std::time::Duration::from_secs(60 * 60))
    }
}

#[async_trait::async_trait]
impl<B: AuthBackend + Send + Sync> AuthBackend for PersistentLockoutBackend<B> {
    type User = B::User;

    async fn authenticate(
        &self,
        db: &djangors_db::Database,
        username: &str,
        password: &str,
    ) -> Result<Option<Self::User>, AuthError> {
        let now = chrono::Utc::now();
        let existing = LoginLockout::objects()
            .filter(djangors_orm::q!(username = username))?
            .first(db)
            .await?;

        // An unexpired lockout rejects the attempt outright, without even calling
        // the inner backend - correct credentials don't help during a lockout.
        if let Some(lockout) = &existing {
            if let Some(locked_until) = lockout.locked_until {
                if locked_until > now {
                    let retry_after_secs = (locked_until - now).num_seconds().max(0) as u64;
                    return Err(AuthError::AccountLocked { retry_after_secs });
                }
            }
        }

        let result = self.inner.authenticate(db, username, password).await;

        // A lockout whose window already expired counts as no active streak for
        // the purposes of deciding whether to start counting from 1 again - but
        // a successful login must still clear the row below regardless, so this
        // is only consulted on the failure branch, not used to gate cleanup.
        let active_streak = existing
            .as_ref()
            .filter(|l| l.locked_until.is_none_or(|u| u > now))
            .is_some();

        match (&result, existing) {
            (Ok(Some(_)), Some(lockout)) => {
                // Successful login clears any existing failure streak, expired or not.
                lockout.delete(db).await?;
            }
            (Ok(Some(_)), None) => {}
            (Ok(None), Some(mut lockout)) if active_streak => {
                lockout.failed_attempts += 1;
                if lockout.failed_attempts as u32 >= self.max_attempts {
                    lockout.locked_until = Some(now + self.lockout_duration);
                }
                lockout.update(db).await?;
            }
            (Ok(None), Some(mut lockout)) => {
                // A row exists but its streak already expired - start a fresh
                // count-of-1 in place, rather than accumulating onto the old streak
                // (or trying to INSERT a second row and hitting the UNIQUE
                // constraint on `username`).
                lockout.failed_attempts = 1;
                lockout.first_failed_at = now;
                lockout.locked_until = if self.max_attempts <= 1 {
                    Some(now + self.lockout_duration)
                } else {
                    None
                };
                lockout.update(db).await?;
            }
            (Ok(None), None) => {
                let locked_until = if self.max_attempts <= 1 {
                    Some(now + self.lockout_duration)
                } else {
                    None
                };
                let lockout = LoginLockout {
                    id: 0,
                    username: username.to_string(),
                    failed_attempts: 1,
                    first_failed_at: now,
                    locked_until,
                };
                lockout.save(db).await?;
            }
            (Err(_), _) => {
                // A backend-level error (DB failure, inner rate-limit, etc.) is not
                // a credential failure - don't count it against the lockout streak.
            }
        }

        result
    }
}

/// Session key used to store the authenticated user ID.
pub const SESSION_USER_ID_KEY: &str = "_auth_user_id";

/// Establish an authenticated session for `user`. Rotates the session
/// identity first (session-fixation protection: an attacker who fixed a
/// session ID before login gains nothing from the now-authenticated
/// session, since its identity changed), then stores the user's id.
pub fn login<U: AuthUser>(session: &djangors_sessions::Session, user: &U) {
    session.cycle_key();
    session.set(SESSION_USER_ID_KEY, user.id());
}

/// Clear the authenticated session. `Session::clear()` already regenerates
/// the session's internal key as part of clearing (see
/// crates/djangors-sessions/src/lib.rs's `clear()`), so this alone already
/// gives logout a fresh, unrelated session identity - no separate
/// `cycle_key()` call needed here.
pub async fn logout(session: &djangors_sessions::Session) {
    let user_id = session.get::<i64>(SESSION_USER_ID_KEY);
    session.clear();
    LOGGED_OUT.send(LoggedOut { user_id }).await;
}

/// Extracts the currently-authenticated user of type `U`.
#[derive(Debug, Clone)]
pub struct Auth<U: AuthUser>(pub U);

#[async_trait::async_trait]
impl<U: AuthUser> djangors_core::extract::FromRequest for Auth<U> {
    async fn from_request(
        req: &djangors_core::Request,
    ) -> Result<Self, djangors_core::DjangorsError> {
        let session = req.ext::<djangors_sessions::Session>().ok_or_else(|| {
            djangors_core::error::DjangorsError::Unauthorized(
                "session extension absent".to_string(),
            )
        })?;

        let user_id = session.get::<i64>(SESSION_USER_ID_KEY).ok_or_else(|| {
            djangors_core::error::DjangorsError::Unauthorized("not authenticated".to_string())
        })?;

        let db = req.state::<djangors_db::Database>().ok_or_else(|| {
            djangors_core::error::DjangorsError::Internal("database state absent".to_string())
        })?;

        let filter_expr = djangors_orm::q!(id = user_id);

        let user = U::objects()
            .filter(filter_expr)
            .map_err(|e| djangors_core::error::DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|e| match e {
                djangors_orm::OrmError::NotFound { .. } => {
                    djangors_core::error::DjangorsError::Unauthorized("user not found".to_string())
                }
                _ => djangors_core::error::DjangorsError::Internal(e.to_string()),
            })?;

        if !user.is_active() {
            return Err(djangors_core::error::DjangorsError::Unauthorized(
                "account inactive".to_string(),
            ));
        }

        let current_gen = get_session_generation(db, user.id())
            .await
            .map_err(|e| djangors_core::error::DjangorsError::Internal(e.to_string()))?;
        if current_gen > 0 {
            let sess_gen = session.get::<i64>(SESSION_GENERATION_KEY).unwrap_or(0);
            if sess_gen != current_gen {
                return Err(djangors_core::error::DjangorsError::Unauthorized(
                    "session invalidated".to_string(),
                ));
            }
        }

        Ok(Auth(user))
    }
}

/// Anything djangors-auth's login/session/permission machinery can operate on.
/// Real apps that need custom user fields implement this on their own struct instead of using the built-in `User`.
#[async_trait::async_trait]
pub trait AuthUser: djangors_orm::Model + djangors_orm::FromRow + Send + Sync + 'static {
    /// Returns the unique primary key identifier for the user.
    fn id(&self) -> i64;
    /// Returns the username of the user.
    fn username(&self) -> &str;
    /// Returns the password hash string of the user.
    fn password_hash(&self) -> &str;
    /// Sets the password hash string of the user.
    fn set_password_hash(&mut self, hash: String);
    /// Returns `true` if the user account is active.
    fn is_active(&self) -> bool;
    /// Returns `true` if the user is a superuser.
    fn is_superuser(&self) -> bool;
    /// Updates the user record in the database.
    async fn update_user(&self, db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError>;
}

/// The default concrete `User` model implementing `AuthUser`.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user")]
pub struct User {
    /// Primary key user identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,

    /// Unique username.
    #[djangors(max_length = 150)]
    pub username: String,

    /// User email address.
    #[djangors(max_length = 254)]
    pub email: String,

    /// PHC-format hash string (algorithm + params + salt + hash all
    /// self-contained) - never the plaintext password.
    pub password: String,

    /// Whether the user account is active.
    pub is_active: bool,
    /// Whether the user has staff privileges.
    pub is_staff: bool,
    /// Whether the user has superuser privileges.
    pub is_superuser: bool,

    /// Account creation timestamp.
    pub date_joined: chrono::DateTime<chrono::Utc>,
    /// Optional last login timestamp.
    pub last_login: Option<chrono::DateTime<chrono::Utc>>,
}

#[async_trait::async_trait]
impl AuthUser for User {
    fn id(&self) -> i64 {
        self.id
    }

    fn username(&self) -> &str {
        &self.username
    }

    fn password_hash(&self) -> &str {
        &self.password
    }

    fn set_password_hash(&mut self, hash: String) {
        self.password = hash;
    }

    fn is_active(&self) -> bool {
        self.is_active
    }

    fn is_superuser(&self) -> bool {
        self.is_superuser
    }

    async fn update_user(&self, db: &djangors_db::Database) -> Result<(), djangors_orm::OrmError> {
        self.update(db).await
    }
}

/// Model representing a permission codename and human-readable name.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_permission")]
pub struct Permission {
    /// Primary key permission identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// "{app_label}.{action}_{model_name_lowercase}", e.g. "polls.add_question".
    #[djangors(max_length = 255, unique)]
    pub codename: String,
    /// Human-readable label, e.g. "Can add question".
    #[djangors(max_length = 255)]
    pub name: String,
}

/// Model representing a user permission group.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_group")]
pub struct Group {
    /// Primary key group identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Unique group name.
    #[djangors(max_length = 150, unique)]
    pub name: String,
}

/// Join table: which groups a user belongs to.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user_groups")]
pub struct UserGroup {
    /// Primary key join identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Foreign key reference to User.
    pub user: ForeignKey<User>,
    /// Foreign key reference to Group.
    pub group: ForeignKey<Group>,
}

/// Join table: which permissions a group grants.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_group_permissions")]
pub struct GroupPermission {
    /// Primary key join identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Foreign key reference to Group.
    pub group: ForeignKey<Group>,
    /// Foreign key reference to Permission.
    pub permission: ForeignKey<Permission>,
}

/// Join table: permissions granted directly to a user (bypassing any group).
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_user_permissions")]
pub struct UserPermission {
    /// Primary key join identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Foreign key reference to User.
    pub user: ForeignKey<User>,
    /// Foreign key reference to Permission.
    pub permission: ForeignKey<Permission>,
}

/// Checks whether `user_id` has been granted `codename`, either directly
/// (`UserPermission`) or via any group they belong to (`UserGroup` ->
/// `GroupPermission`). Does NOT special-case superusers - callers that want
/// "superusers can do anything" should check `user.is_superuser()` first and
/// skip calling this entirely, avoiding a DB round-trip for the common case.
pub async fn has_perm(
    db: &djangors_db::Database,
    user_id: i64,
    codename: &str,
) -> Result<bool, AuthError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let sql1 = format!(
        "SELECT COUNT(*) FROM auth_user_permissions up \
         JOIN auth_permission p ON p.id = up.permission \
         WHERE up.\"user\" = {p1} AND p.codename = {p2}"
    );
    let params1 = vec![
        djangors_db::BindValue::I64(user_id),
        djangors_db::BindValue::Text(codename.to_string()),
    ];
    let row = conn
        .fetch_one(&sql1, &params1)
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let direct = row
        .try_i64(0)
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?
        .unwrap_or(0);
    if direct > 0 {
        return Ok(true);
    }

    let sql2 = format!(
        "SELECT COUNT(*) FROM auth_user_groups ug \
         JOIN auth_group_permissions gp ON gp.\"group\" = ug.\"group\" \
         JOIN auth_permission p ON p.id = gp.permission \
         WHERE ug.\"user\" = {p1} AND p.codename = {p2}"
    );
    let params2 = vec![
        djangors_db::BindValue::I64(user_id),
        djangors_db::BindValue::Text(codename.to_string()),
    ];
    let row2 = conn
        .fetch_one(&sql2, &params2)
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let via_group = row2
        .try_i64(0)
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?
        .unwrap_or(0);
    Ok(via_group > 0)
}

/// Ensures the 4 standard permissions (view/add/change/delete) exist for
/// every model currently registered via `djangors_orm::meta::all_registered_models()`.
/// This covers every `#[derive(Model)]` struct in the binary, using the same pre-existing
/// global registry that 5.5's `collect_related_objects` already established as
/// the source of truth for "every model in the project." Idempotent: safe
/// to call on every app startup or as a repeatable CLI step. Returns the
/// number of (model, action) pairs considered (not the number newly
/// inserted - `ON CONFLICT DO NOTHING` makes re-runs cheap no-ops without
/// needing to track that distinction).
pub async fn sync_permissions(db: &djangors_db::Database) -> Result<usize, AuthError> {
    let mut count = 0;
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let sql = format!(
        "INSERT INTO auth_permission (codename, name) VALUES ({p1}, {p2}) \
         ON CONFLICT (codename) DO NOTHING"
    );
    for meta in djangors_orm::meta::all_registered_models() {
        for action in ["view", "add", "change", "delete"] {
            let codename = format!(
                "{}.{}_{}",
                meta.app_label,
                action,
                meta.struct_name.to_lowercase()
            );
            let name = format!("Can {} {}", action, meta.struct_name.to_lowercase());
            let params = vec![
                djangors_db::BindValue::Text(codename),
                djangors_db::BindValue::Text(name),
            ];
            conn.execute(&sql, &params)
                .await
                .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
            count += 1;
        }
    }
    Ok(count)
}

/// Hash a plaintext password using Argon2id and generate a random salt.
/// Returns a PHC-formatted hash string.
///
/// For legacy verifiers (e.g. supporting old hash formats when migrating an existing user base),
/// this function or a future `PasswordHasher` trait can be extended. For now, it defaults to Argon2id.
pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AuthError::Hashing(e.to_string()))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a PHC-formatted Argon2id hash.
/// Returns `Ok(true)` if valid, `Ok(false)` if the password doesn't match,
/// or `Err(AuthError)` if the hash string is malformed or corrupted.
pub fn verify_password(password: &str, phc_hash: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(phc_hash).map_err(|e| AuthError::Hashing(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// Generates a signed password reset token.
///
/// The token embeds the user's ID, an expiry timestamp, and a prefix of the user's current password hash.
/// This ensures that the token becomes invalid as soon as the user's password changes.
pub fn generate_password_reset_token<U: AuthUser>(
    user: &U,
    secret: &[u8],
    ttl: Duration,
) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let expiry_unix_secs = (now + ttl).as_secs();

    let user_id_str = user.id().to_string();
    let b64_user_id = base64::engine::general_purpose::STANDARD.encode(user_id_str.as_bytes());
    let b64_expiry =
        base64::engine::general_purpose::STANDARD.encode(expiry_unix_secs.to_string().as_bytes());

    let hash = user.password_hash();
    let prefix_len = std::cmp::min(30, hash.len());
    let password_hash_prefix = &hash[..prefix_len];

    let msg = format!(
        "{}.{}.{}",
        user.id(),
        expiry_unix_secs,
        password_hash_prefix
    );

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg.as_bytes());
    let mac_result = mac.finalize().into_bytes();
    let b64_mac = base64::engine::general_purpose::STANDARD.encode(mac_result);

    format!("{}.{}.{}", b64_user_id, b64_expiry, b64_mac)
}

/// Verifies a signed password reset token against a user.
///
/// Returns `true` if the token is valid, has not expired, was generated for the correct user,
/// and the user's password hash has not changed since the token was generated.
pub fn verify_password_reset_token<U: AuthUser>(user: &U, token: &str, secret: &[u8]) -> bool {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    let b64_user_id = parts[0];
    let b64_expiry = parts[1];
    let b64_mac = parts[2];

    // Decode and verify user_id
    let user_id_bytes = match base64::engine::general_purpose::STANDARD.decode(b64_user_id) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let user_id_str = match std::str::from_utf8(&user_id_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let token_user_id: i64 = match user_id_str.parse() {
        Ok(id) => id,
        Err(_) => return false,
    };
    if token_user_id != user.id() {
        return false;
    }

    // Decode and verify expiry
    let expiry_bytes = match base64::engine::general_purpose::STANDARD.decode(b64_expiry) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let expiry_str = match std::str::from_utf8(&expiry_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let expiry_unix_secs: u64 = match expiry_str.parse() {
        Ok(t) => t,
        Err(_) => return false,
    };
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        Err(_) => 0,
    };
    if expiry_unix_secs <= now {
        return false;
    }

    // Verify HMAC using the user's current password hash prefix
    let hash = user.password_hash();
    let prefix_len = std::cmp::min(30, hash.len());
    let password_hash_prefix = &hash[..prefix_len];

    let msg = format!(
        "{}.{}.{}",
        user.id(),
        expiry_unix_secs,
        password_hash_prefix
    );

    let mac_bytes = match base64::engine::general_purpose::STANDARD.decode(b64_mac) {
        Ok(b) => b,
        Err(_) => return false,
    };

    let mut mac = match HmacSha256::new_from_slice(secret) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(msg.as_bytes());
    mac.verify_slice(&mac_bytes).is_ok()
}

/// Looks up an active user by email and, if found, sends a password reset link to them.
///
/// To prevent user enumeration attacks, this function always returns `Ok(())` and runs in
/// a similar time frame regardless of whether the email was registered.
pub async fn request_password_reset<U: AuthUser>(
    db: &djangors_db::Database,
    mail: &dyn djangors_mail::MailBackend,
    email: &str,
    secret: &[u8],
    reset_link_base: &str,
) -> Result<(), AuthError> {
    let mut users = U::objects()
        .filter(djangors_orm::q!(email = email))?
        .all(db)
        .await?;

    let user_opt = if users.len() == 1 {
        let u = users.remove(0);
        if u.is_active() {
            Some(u)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(user) = user_opt {
        // Use a default 1-hour TTL for the token.
        let token = generate_password_reset_token(&user, secret, Duration::from_secs(3600));
        let reset_link = format!("{}{}", reset_link_base, token);
        let message = djangors_mail::Message::new(
            [email.to_string()],
            "noreply@localhost",
            "Password Reset Request",
            format!(
                "You are receiving this email because you requested a password reset for your user account.\n\
                 Please click the following link to choose a new password:\n\n\
                 {}\n\n\
                 If you did not request this reset, you can safely ignore this email.",
                reset_link
            ),
        );
        mail.send(&message)
            .await
            .map_err(|e| AuthError::Mail(e.to_string()))?;
    }

    Ok(())
}

/// Confirms the password reset by verifying the token, hashing the new password, and updating the database.
pub async fn confirm_password_reset<U: AuthUser>(
    db: &djangors_db::Database,
    user_id: i64,
    token: &str,
    new_password: &str,
    secret: &[u8],
) -> Result<(), AuthError> {
    let mut users = U::objects()
        .filter(djangors_orm::q!(id = user_id))?
        .all(db)
        .await?;

    if users.len() != 1 {
        return Err(AuthError::InvalidToken);
    }
    let mut user = users.remove(0);

    if !verify_password_reset_token(&user, token, secret) {
        return Err(AuthError::InvalidToken);
    }

    let attrs = UserAttributes::new(user.username(), "");
    if let Err(errors) = validate_password(new_password, Some(&attrs), &default_validators()) {
        return Err(AuthError::WeakPassword(errors));
    }

    let hashed = hash_password(new_password)?;
    user.set_password_hash(hashed);
    user.update_user(db).await?;
    bump_session_generation(db, user.id()).await?;

    Ok(())
}

#[cfg(test)]
mod tests;
