//! Passwordless magic-link login.
//!
//! [`send_login_link`] emails a single-use sign-in link. The link opens a confirmation page
//! with a POST button, because mail scanners fetch GET links and would otherwise burn them.
//! [`consume_login_link`] claims a token with one conditional `UPDATE`, so a token works once
//! even under concurrent requests.
//!
//! Tokens are 256-bit random values. Only their SHA-256 hash is stored. Sending is throttled
//! per normalized email and per client IP, and the throttle is stored in the database.

use crate::{AuthError, LoginOptions, User};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_db::{BindValue, Database};
use djangors_macros::Model;
use djangors_mail::{MailBackend, Message};
use djangors_orm::{ForeignKey, Model as _};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::time::Duration;
use subtle::ConstantTimeEq;

/// Single-use passwordless login link record.
///
/// Only the SHA-256 hash of the raw token is stored (`token_hash`).
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_login_link")]
pub struct LoginLink {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// User authenticated by this login link.
    pub user: ForeignKey<User>,
    /// SHA-256 hex digest of the raw token.
    #[djangors(max_length = 128, unique)]
    pub token_hash: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
    /// Timestamp when the link was consumed, if used.
    pub used_at: Option<DateTime<Utc>>,
}

/// Default validity of a login link (15 minutes).
pub const DEFAULT_LOGIN_LINK_TTL: Duration = Duration::from_secs(15 * 60);

/// Limits applied to [`send_login_link`].
#[derive(Debug, Clone, Copy)]
pub struct LoginLinkPolicy {
    /// How long a link stays valid.
    pub ttl: Duration,
    /// Links that may be requested for one email address within `window`.
    pub max_per_email: u32,
    /// Requests that may come from one client IP within `window`.
    pub max_per_ip: u32,
    /// Length of the throttling window.
    pub window: Duration,
}

impl Default for LoginLinkPolicy {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_LOGIN_LINK_TTL,
            max_per_email: 5,
            max_per_ip: 20,
            window: Duration::from_secs(60 * 60),
        }
    }
}

/// Creates the `auth_login_link` and `auth_login_link_attempt` tables.
///
/// Call this once at startup or in a migration. It is not run per request.
pub async fn create_login_link_tables(db: &Database) -> Result<(), AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let bigint = dialect.bigint_type();
    let ts = dialect.timestamp_type();
    let links = format!(
        "CREATE TABLE IF NOT EXISTS auth_login_link (
            id {pk},
            \"user\" {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
            token_hash VARCHAR(128) NOT NULL UNIQUE,
            created_at {ts} NOT NULL,
            expires_at {ts} NOT NULL,
            used_at {ts}
        )"
    );
    conn.execute(&links, &[])
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let attempts = format!(
        "CREATE TABLE IF NOT EXISTS auth_login_link_attempt (
            id {pk},
            scope VARCHAR(160) NOT NULL,
            created_at {ts} NOT NULL
        )"
    );
    conn.execute(&attempts, &[])
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS auth_login_link_attempt_scope
         ON auth_login_link_attempt (scope, created_at)",
        &[],
    )
    .await
    .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn generate_raw_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

/// SHA-256 of a raw login token, hex encoded. This is what the database stores.
pub fn hash_login_token(raw_token: &str) -> String {
    hex(&Sha256::digest(raw_token.as_bytes()))
}

/// Records one attempt under `scope` and fails with [`AuthError::RateLimited`] when more than
/// `max` attempts fall inside `window`.
async fn throttle(db: &Database, scope: &str, max: u32, window: Duration) -> Result<(), AuthError> {
    let now = Utc::now();
    let since = now - ChronoDuration::from_std(window).unwrap_or_default();
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    conn.execute(
        &format!("INSERT INTO auth_login_link_attempt (scope, created_at) VALUES ({p1}, {p2})"),
        &[BindValue::Text(scope.to_owned()), BindValue::DateTime(now)],
    )
    .await
    .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let row = conn
        .fetch_optional(
            &format!(
                "SELECT COUNT(*) FROM auth_login_link_attempt
                 WHERE scope = {p1} AND created_at > {p2}"
            ),
            &[
                BindValue::Text(scope.to_owned()),
                BindValue::DateTime(since),
            ],
        )
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let count = row.and_then(|r| r.try_i64(0).ok().flatten()).unwrap_or(0);
    if count > i64::from(max) {
        return Err(AuthError::RateLimited);
    }
    Ok(())
}

/// Emails a sign-in link to the active user with address `email`.
///
/// The function returns `Ok(())` whether or not the address belongs to a user, so callers can
/// show the same message in both cases. Only a throttle hit returns an error. The raw token
/// is never logged and is sent only in the email body.
pub async fn send_login_link(
    db: &Database,
    mailer: &dyn MailBackend,
    email: &str,
    client_ip: Option<&str>,
    base_url: &str,
    policy: LoginLinkPolicy,
) -> Result<(), AuthError> {
    let normalized = email.trim().to_ascii_lowercase();

    throttle(
        db,
        &format!("email:{}", hash_login_token(&normalized)),
        policy.max_per_email,
        policy.window,
    )
    .await?;
    if let Some(ip) = client_ip {
        throttle(db, &format!("ip:{ip}"), policy.max_per_ip, policy.window).await?;
    }

    let user = User::objects()
        .filter(djangors_orm::q!(email = normalized.as_str()))?
        .all(db)
        .await?
        .into_iter()
        .find(|u| u.is_active);
    let Some(user) = user else {
        return Ok(());
    };

    let raw = generate_raw_token();
    let now = Utc::now();
    let expires = now + ChronoDuration::from_std(policy.ttl).unwrap_or_default();
    let link_row = LoginLink {
        id: 0,
        user: ForeignKey::new(user.id),
        token_hash: hash_login_token(&raw),
        created_at: now,
        expires_at: expires,
        used_at: None,
    };
    let _link_row = link_row.save(db).await?;

    let link = format!("{}/login/magic/{}", base_url.trim_end_matches('/'), raw);
    let msg = Message::new(
        [normalized],
        "noreply@localhost",
        "Your sign-in link",
        format!(
            "Use this link to sign in. It works once and expires in {} minutes:\n\n{link}\n\n\
             If you did not ask for it, ignore this email.\n",
            policy.ttl.as_secs() / 60
        ),
    );
    mailer
        .send(&msg)
        .await
        .map_err(|e| AuthError::Mail(e.to_string()))?;
    Ok(())
}

/// Claims a login token and returns its user.
///
/// The claim is a single conditional `UPDATE ... WHERE used_at IS NULL AND expires_at > now`.
/// Exactly one caller can win it. Unknown, expired, used and inactive-user tokens all return
/// [`AuthError::InvalidToken`].
pub async fn consume_login_link(db: &Database, raw_token: &str) -> Result<User, AuthError> {
    if raw_token.trim().is_empty() {
        return Err(AuthError::InvalidToken);
    }
    let token_hash = hash_login_token(raw_token);
    let now = Utc::now();
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let p3 = conn.dialect().placeholder(3);
    let claimed = conn
        .execute(
            &format!(
                "UPDATE auth_login_link SET used_at = {p1}
                 WHERE token_hash = {p2} AND used_at IS NULL AND expires_at > {p3}"
            ),
            &[
                BindValue::DateTime(now),
                BindValue::Text(token_hash.clone()),
                BindValue::DateTime(now),
            ],
        )
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    if claimed != 1 {
        return Err(AuthError::InvalidToken);
    }
    let p1 = conn.dialect().placeholder(1);
    let row = conn
        .fetch_optional(
            &format!("SELECT \"user\", token_hash FROM auth_login_link WHERE token_hash = {p1}"),
            &[BindValue::Text(token_hash.clone())],
        )
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    let row = row.ok_or(AuthError::InvalidToken)?;
    let stored_hash = row
        .try_string(1)
        .ok()
        .flatten()
        .ok_or(AuthError::InvalidToken)?;
    if !bool::from(stored_hash.as_bytes().ct_eq(token_hash.as_bytes())) {
        return Err(AuthError::InvalidToken);
    }
    let user_id = row
        .try_i64(0)
        .ok()
        .flatten()
        .ok_or(AuthError::InvalidToken)?;
    let user = User::objects()
        .filter(djangors_orm::q!(id = user_id))?
        .all(db)
        .await?
        .into_iter()
        .next()
        .ok_or(AuthError::InvalidToken)?;
    if !user.is_active {
        return Err(AuthError::InvalidToken);
    }
    Ok(user)
}

/// Reads the `csrftoken` cookie, if any, so the confirmation form can pass CSRF checks.
fn csrf_cookie(req: &Request) -> Option<String> {
    let header = req.header("cookie")?.to_str().ok()?;
    header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == "csrftoken")
        .map(|(_, value)| value.to_owned())
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// `GET /login/magic/{token}`: shows a confirmation page with a POST button.
///
/// The GET itself never signs anyone in, so mail scanners that prefetch links are harmless.
pub async fn magic_link_confirm_view(
    req: Request,
    params: PathParams,
) -> Result<Response, DjangorsError> {
    let token = params.get("token").ok_or_else(|| {
        DjangorsError::api(StatusCode::BAD_REQUEST, "missing_token", "Missing token")
    })?;
    let action = djangors_core::html_escape(&format!("/login/magic/{token}"));
    let csrf_token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .or_else(|| csrf_cookie(&req));
    let csrf_field = csrf_token
        .map(|t| {
            format!(
                "<input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{}\">",
                djangors_core::html_escape(&t)
            )
        })
        .unwrap_or_default();
    Ok(Response::html(
        StatusCode::OK,
        format!(
            "<!doctype html><html><body><h1>Sign in</h1>\
             <form method=\"post\" action=\"{action}\">{csrf_field}\
             <button type=\"submit\">Sign in</button></form></body></html>"
        ),
    )
    .header("referrer-policy", "no-referrer")
    .header("cache-control", "no-store"))
}

/// `POST /login/magic/{token}`: consumes the link and logs the user in.
///
/// Requires the session middleware. An invalid or used link gets `400`.
pub async fn magic_link_consume_view(
    req: Request,
    params: PathParams,
) -> Result<Response, DjangorsError> {
    let token = params.get("token").ok_or_else(|| {
        DjangorsError::api(StatusCode::BAD_REQUEST, "missing_token", "Missing token")
    })?;
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
    let user = match consume_login_link(db, token).await {
        Ok(user) => user,
        Err(AuthError::InvalidToken) => {
            return Err(DjangorsError::api(
                StatusCode::BAD_REQUEST,
                "invalid_login_link",
                "This sign-in link is invalid, expired or already used",
            ));
        }
        Err(e) => return Err(e.into()),
    };
    crate::login_with(&req, &user, LoginOptions::new(false)).await?;
    Ok(Response::redirect("/"))
}

/// Mountable router for the magic-link confirmation and consume endpoints.
pub fn magic_link_router() -> djangors_core::Router {
    djangors_core::Router::new()
        .get("/login/magic/{token}", magic_link_confirm_view)
        .post("/login/magic/{token}", magic_link_consume_view)
}

#[cfg(test)]
#[path = "magic_link_tests.rs"]
mod magic_link_tests;
