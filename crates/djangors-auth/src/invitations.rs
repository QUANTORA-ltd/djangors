//! User invitation workflow (`Invitation` model, token rotation, accept/revoke, and views).

use crate::{
    default_validators, hash_password, validate_password, Auth, AuthError, User, UserAttributes,
};
use chrono::{DateTime, Utc};
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_mail::{MailBackend, Message};
use djangors_orm::{ForeignKey, Model as _};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// Invitation record for onboarding new or existing users (optionally into an organization).
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_invitation")]
pub struct Invitation {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Invited email address.
    #[djangors(max_length = 254)]
    pub email: String,
    /// User who sent the invitation.
    pub inviter: ForeignKey<User>,
    /// SHA-256 hex digest of the invitation token.
    #[djangors(max_length = 128)]
    pub token_hash: String,
    /// Optional organization ID (used by `djangors-contrib-orgs`).
    pub organization_id: Option<i64>,
    /// Role assigned when the invitation is accepted.
    #[djangors(max_length = 64)]
    pub role: String,
    /// Expiration timestamp (UTC).
    pub expires_at: DateTime<Utc>,
    /// Timestamp when the invitation was accepted, if accepted.
    pub accepted_at: Option<DateTime<Utc>>,
    /// Whether this invitation has been revoked.
    pub revoked: bool,
}

/// Ensures the `auth_invitation` table exists.
pub async fn create_invitation_table(db: &Database) -> Result<(), AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let bigint = dialect.bigint_type();
    let ts = dialect.timestamp_type();
    let bool_ty = dialect.bool_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS auth_invitation (
            id {pk},
            email VARCHAR(254) NOT NULL,
            inviter {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
            token_hash VARCHAR(128) NOT NULL UNIQUE,
            organization_id {bigint},
            role VARCHAR(64) NOT NULL,
            expires_at {ts} NOT NULL,
            accepted_at {ts},
            revoked {bool_ty} NOT NULL DEFAULT FALSE
        )"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    Ok(())
}

fn generate_raw_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Hashes a raw invitation token with SHA-256.
pub fn hash_invitation_token(raw_token: &str) -> String {
    let digest = Sha256::digest(raw_token.as_bytes());
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// Creates a new [`Invitation`], stores its hashed token, and emails the accept link.
/// Returns `(invitation, raw_token)`.
#[allow(clippy::too_many_arguments)]
pub async fn invite(
    db: &Database,
    mailer: &dyn MailBackend,
    inviter_id: i64,
    email: &str,
    organization_id: Option<i64>,
    role: &str,
    base_url: &str,
    ttl: Duration,
) -> Result<(Invitation, String), AuthError> {
    create_invitation_table(db).await?;
    let normalized = email.trim().to_ascii_lowercase();
    let raw_token = generate_raw_token();
    let token_hash = hash_invitation_token(&raw_token);
    let expires_at = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();

    let inv = Invitation {
        id: 0,
        email: normalized.clone(),
        inviter: ForeignKey::new(inviter_id),
        token_hash,
        organization_id,
        role: role.to_string(),
        expires_at,
        accepted_at: None,
        revoked: false,
    };
    let inv = inv.save(db).await?;

    send_invitation_email(mailer, &normalized, &raw_token, base_url).await?;
    Ok((inv, raw_token))
}

/// Resends an unaccepted, non-revoked invitation, rotating its token and extending `expires_at`.
pub async fn resend_invitation(
    db: &Database,
    mailer: &dyn MailBackend,
    invitation_id: i64,
    base_url: &str,
    ttl: Duration,
) -> Result<(Invitation, String), AuthError> {
    create_invitation_table(db).await?;
    let mut inv = Invitation::objects()
        .filter(djangors_orm::q!(id = invitation_id))?
        .get(db)
        .await?;
    if inv.revoked || inv.accepted_at.is_some() {
        return Err(AuthError::InvalidToken);
    }
    let raw_token = generate_raw_token();
    inv.token_hash = hash_invitation_token(&raw_token);
    inv.expires_at = Utc::now() + chrono::Duration::from_std(ttl).unwrap_or_default();
    inv.update(db).await?;

    send_invitation_email(mailer, &inv.email, &raw_token, base_url).await?;
    Ok((inv, raw_token))
}

async fn send_invitation_email(
    mailer: &dyn MailBackend,
    email: &str,
    raw_token: &str,
    base_url: &str,
) -> Result<(), AuthError> {
    let link = format!(
        "{}/invitations/accept?token={}",
        base_url.trim_end_matches('/'),
        raw_token
    );
    let msg = Message::new(
        [email.to_string()],
        "noreply@localhost",
        "You have been invited",
        format!("Accept your invitation by visiting:\n\n{link}\n"),
    );
    mailer
        .send(&msg)
        .await
        .map_err(|e| AuthError::Mail(e.to_string()))?;
    Ok(())
}

/// Revokes an invitation by ID so it can no longer be accepted.
pub async fn revoke_invitation(db: &Database, invitation_id: i64) -> Result<(), AuthError> {
    create_invitation_table(db).await?;
    let mut inv = Invitation::objects()
        .filter(djangors_orm::q!(id = invitation_id))?
        .get(db)
        .await?;
    inv.revoked = true;
    inv.update(db).await?;
    Ok(())
}

/// Accepts an invitation token.
///
/// - If no user exists for `invitation.email`, creates a new active [`User`] using `password`
///   (returning `WeakPassword` if `password` is missing or weak).
/// - If a user already exists for `invitation.email`, `current_user` must be `Some(user)`
///   matching that email; otherwise returns `Err(AuthError::InvitationEmailMismatch)`.
/// - Marks `accepted_at` and returns `(user, invitation)`.
pub async fn accept_invitation(
    db: &Database,
    token: &str,
    password: Option<&str>,
    current_user: Option<&User>,
) -> Result<(User, Invitation), AuthError> {
    create_invitation_table(db).await?;
    let token_hash = hash_invitation_token(token);
    let mut rows = Invitation::objects()
        .filter(djangors_orm::q!(token_hash = token_hash.as_str()))?
        .all(db)
        .await?;
    let mut inv = rows.pop().ok_or(AuthError::InvalidToken)?;

    if inv.revoked || inv.accepted_at.is_some() || inv.expires_at <= Utc::now() {
        return Err(AuthError::InvalidToken);
    }

    let existing_users = User::objects()
        .filter(djangors_orm::q!(email = inv.email.as_str()))?
        .all(db)
        .await?;

    let user = if let Some(existing) = existing_users.into_iter().next() {
        match current_user {
            Some(cu) if cu.email.trim().eq_ignore_ascii_case(&inv.email) => existing,
            _ => return Err(AuthError::InvitationEmailMismatch),
        }
    } else {
        let raw_pw = password.ok_or_else(|| {
            AuthError::WeakPassword(vec!["password is required for new accounts".to_string()])
        })?;
        let attrs = UserAttributes::new(&inv.email, &inv.email);
        if let Err(errors) = validate_password(raw_pw, Some(&attrs), &default_validators()) {
            return Err(AuthError::WeakPassword(errors));
        }
        let hashed = hash_password(raw_pw)?;
        let new_user = User {
            id: 0,
            username: inv.email.clone(),
            email: inv.email.clone(),
            password: hashed,
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: Utc::now(),
            last_login: None,
        };
        let new_user = new_user.save(db).await?;
        new_user
    };

    inv.accepted_at = Some(Utc::now());
    inv.update(db).await?;

    Ok((user, inv))
}

fn html_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders a minimal HTML accept-invitation page (`GET /invitations/accept?token=...`).
///
/// The form carries the CSRF token when [`djangors_core::middleware::CsrfLayer`] is installed.
pub async fn accept_invitation_page(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let token = html_escape(req.query("token").unwrap_or(""));
    let csrf = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| {
            format!(
                "<input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{}\">",
                html_escape(&t.0)
            )
        })
        .unwrap_or_default();
    let html = format!(
        "<!doctype html><html><body><h1>Accept Invitation</h1>\
         <form method=\"post\" action=\"/invitations/accept\">{csrf}\
         <input type=\"hidden\" name=\"token\" value=\"{token}\">\
         <label>Password (new users): <input type=\"password\" name=\"password\" autocomplete=\"new-password\"></label>\
         <button type=\"submit\">Accept</button>\
         </form></body></html>"
    );
    Ok(Response::html(StatusCode::OK, html).header("referrer-policy", "no-referrer"))
}

/// Handles the accept form (`POST /invitations/accept`, form fields `token` and `password`).
///
/// A logged-in user whose email matches the invitation accepts it for their existing
/// account; otherwise a new account is created with the submitted password.
pub async fn accept_invitation_submit(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let fields: std::collections::HashMap<String, String> =
        serde_urlencoded::from_bytes(req.body_bytes().await).map_err(|_| {
            DjangorsError::api(StatusCode::BAD_REQUEST, "invalid_form", "Invalid form body")
        })?;
    let token = fields.get("token").map(String::as_str).unwrap_or("");
    let password = fields
        .get("password")
        .map(String::as_str)
        .filter(|p| !p.is_empty());
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
    let current = <Auth<User> as djangors_core::extract::FromRequest>::from_request(&req)
        .await
        .ok()
        .map(|Auth(u)| u);
    accept_invitation(db, token, password, current.as_ref()).await?;
    Ok(Response::html(
        StatusCode::OK,
        "<!doctype html><html><body><h1>Invitation accepted</h1>\
         <p>You can now <a href=\"/login\">log in</a>.</p></body></html>",
    ))
}

/// Lists all invitations as JSON for staff/admin users (`GET /admin/invitations`).
pub async fn admin_invitations_list(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let Auth(user) =
        <Auth<User> as djangors_core::extract::FromRequest>::from_request(&req).await?;
    if !user.is_staff {
        return Err(DjangorsError::Forbidden(
            "staff access required".to_string(),
        ));
    }
    let db = req
        .state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
    create_invitation_table(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let items = Invitation::objects()
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let payload: Vec<serde_json::Value> = items
        .into_iter()
        .map(|i| {
            serde_json::json!({
                "id": i.id,
                "email": i.email,
                "organization_id": i.organization_id,
                "role": i.role,
                "expires_at": i.expires_at,
                "accepted_at": i.accepted_at,
                "revoked": i.revoked,
            })
        })
        .collect();
    Response::json(StatusCode::OK, &payload)
}

/// Mountable router exposing the invitation accept page and admin list endpoint.
pub fn invitation_router() -> Router {
    Router::new()
        .get("/invitations/accept", accept_invitation_page)
        .post("/invitations/accept", accept_invitation_submit)
        .get("/admin/invitations", admin_invitations_list)
}
