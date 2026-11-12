//! Email verification workflow, `EmailAddress` model, signed verification tokens,
//! and `HasVerifiedEmail` enforcement.

use crate::{Auth, AuthError, AuthUser, User};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use djangors_core::settings::DjangorsSettings;
use djangors_core::{DjangorsError, Request, StatusCode};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_mail::{MailBackend, Message};
use djangors_orm::{ForeignKey, Model as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Default verification link validity (24 hours).
const DEFAULT_VERIFICATION_TTL_SECS: u64 = 24 * 60 * 60;

/// Domain-separation prefix so a verification token's MAC can never be replayed as
/// another token type signed with the same `SECRET_KEY`.
const EMAIL_VERIFY_PURPOSE: &str = "djangors.auth.email-verify";

/// Email address record associated with a [`User`].
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_auth", table_name = "auth_email_address")]
pub struct EmailAddress {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Owning user.
    pub user: ForeignKey<User>,
    /// Email address string.
    #[djangors(max_length = 254)]
    pub email: String,
    /// Whether this email address has been verified.
    pub verified: bool,
    /// Whether this is the user's primary email address.
    pub primary: bool,
    /// Timestamp when verification completed.
    pub verified_at: Option<DateTime<Utc>>,
}

/// Ensures the `auth_email_address` table exists with a unique `(user_id, email)` constraint.
pub async fn create_email_address_table(db: &Database) -> Result<(), AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let bigint = dialect.bigint_type();
    let bool_ty = dialect.bool_type();
    let ts = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS auth_email_address (
            id {pk},
            \"user\" {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
            email VARCHAR(254) NOT NULL,
            verified {bool_ty} NOT NULL DEFAULT FALSE,
            \"primary\" {bool_ty} NOT NULL DEFAULT FALSE,
            verified_at {ts},
            UNIQUE (\"user\", email)
        )"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    Ok(())
}

/// Generates an HMAC-SHA256 signed, time-limited verification token bound to `(user_id, email)`.
pub fn generate_email_verification_token(
    user_id: i64,
    email: &str,
    secret: &[u8],
    ttl: Duration,
) -> String {
    let expires_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        .saturating_add(ttl.as_secs());

    let normalized_email = email.trim().to_ascii_lowercase();
    let msg = format!("{EMAIL_VERIFY_PURPOSE}:{user_id}.{normalized_email}.{expires_at}");

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg.as_bytes());
    let sig = mac.finalize().into_bytes();

    let eng = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    format!(
        "{}.{}.{}.{}",
        eng.encode(user_id.to_string().as_bytes()),
        eng.encode(normalized_email.as_bytes()),
        eng.encode(expires_at.to_string().as_bytes()),
        eng.encode(sig)
    )
}

/// Decodes and verifies a signed email verification token, returning `(user_id, email)` if valid
/// and unexpired.
pub fn parse_email_verification_token(token: &str, secret: &[u8]) -> Option<(i64, String)> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let eng = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let uid_bytes = eng.decode(parts[0]).ok()?;
    let email_bytes = eng.decode(parts[1]).ok()?;
    let exp_bytes = eng.decode(parts[2]).ok()?;
    let sig_bytes = eng.decode(parts[3]).ok()?;

    let uid_str = std::str::from_utf8(&uid_bytes).ok()?;
    let user_id: i64 = uid_str.parse().ok()?;
    let email = std::str::from_utf8(&email_bytes).ok()?.to_string();
    let exp_str = std::str::from_utf8(&exp_bytes).ok()?;
    let expires_at: u64 = exp_str.parse().ok()?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    if expires_at <= now {
        return None;
    }

    let msg = format!("{EMAIL_VERIFY_PURPOSE}:{user_id}.{email}.{expires_at}");
    let mut mac = HmacSha256::new_from_slice(secret).ok()?;
    mac.update(msg.as_bytes());
    mac.verify_slice(&sig_bytes).ok()?;

    Some((user_id, email))
}

/// Records (or ensures) an unverified [`EmailAddress`] row for `(user_id, email)` and sends
/// a signed verification link to `email`.
pub async fn send_verification_email(
    db: &Database,
    mailer: &dyn MailBackend,
    user_id: i64,
    email: &str,
    base_url: &str,
    secret: &[u8],
) -> Result<String, AuthError> {
    create_email_address_table(db).await?;
    let normalized = email.trim().to_ascii_lowercase();

    let existing = EmailAddress::objects()
        .filter(djangors_orm::q!(
            user = user_id,
            email = normalized.as_str()
        ))?
        .all(db)
        .await?;

    if existing.is_empty() {
        let has_any = !EmailAddress::objects()
            .filter(djangors_orm::q!(user = user_id))?
            .all(db)
            .await?
            .is_empty();
        let record = EmailAddress {
            id: 0,
            user: ForeignKey::new(user_id),
            email: normalized.clone(),
            verified: false,
            primary: !has_any,
            verified_at: None,
        };
        record.save(db).await?;
    }

    let token = generate_email_verification_token(
        user_id,
        &normalized,
        secret,
        Duration::from_secs(DEFAULT_VERIFICATION_TTL_SECS),
    );
    let link = format!(
        "{}/verify-email?token={}",
        base_url.trim_end_matches('/'),
        token
    );

    let msg = Message::new(
        [normalized],
        "noreply@localhost",
        "Verify your email address",
        format!("Please verify your email address by visiting the following link:\n\n{link}\n"),
    );
    mailer
        .send(&msg)
        .await
        .map_err(|e| AuthError::Mail(e.to_string()))?;

    Ok(token)
}

/// Verifies a signed email verification token and marks the matching [`EmailAddress`] verified.
///
/// Rejects expired tokens and rejects tokens whose email does not match the user's current
/// `EmailAddress` record (e.g. a token issued for an old email address after an email change).
pub async fn verify_email(
    db: &Database,
    token: &str,
    secret: &[u8],
) -> Result<EmailAddress, AuthError> {
    create_email_address_table(db).await?;
    let (user_id, token_email) =
        parse_email_verification_token(token, secret).ok_or(AuthError::InvalidToken)?;

    let user = User::objects()
        .filter(djangors_orm::q!(id = user_id))?
        .get(db)
        .await
        .map_err(|_| AuthError::InvalidToken)?;

    if !user.email.trim().eq_ignore_ascii_case(&token_email) {
        return Err(AuthError::InvalidToken);
    }

    let mut rows = EmailAddress::objects()
        .filter(djangors_orm::q!(
            user = user_id,
            email = token_email.as_str()
        ))?
        .all(db)
        .await?;
    let mut record = rows.pop().ok_or(AuthError::InvalidToken)?;
    record.verified = true;
    record.verified_at = Some(Utc::now());
    record.update(db).await?;

    Ok(record)
}

/// Returns `true` if `user_id` has at least one verified [`EmailAddress`].
pub async fn has_verified_email(db: &Database, user_id: i64) -> Result<bool, AuthError> {
    create_email_address_table(db).await?;
    let rows = EmailAddress::objects()
        .filter(djangors_orm::q!(user = user_id, verified = true))?
        .all(db)
        .await?;
    Ok(!rows.is_empty())
}

/// Enforces `settings.require_verified_email_for_login` before completing login.
pub async fn ensure_verified_for_login<U: AuthUser>(
    db: &Database,
    settings: &DjangorsSettings,
    user: &U,
) -> Result<(), AuthError> {
    if settings.require_verified_email_for_login && !has_verified_email(db, user.id()).await? {
        return Err(AuthError::EmailNotVerified);
    }
    Ok(())
}

/// Extractor / permission guard that requires the authenticated [`User`] to have a
/// verified [`EmailAddress`]. Returns `403 Forbidden` otherwise.
#[derive(Debug, Clone)]
pub struct HasVerifiedEmail(pub User);

#[async_trait::async_trait]
impl djangors_core::extract::FromRequest for HasVerifiedEmail {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let Auth(user) = Auth::<User>::from_request(req).await?;
        let db = req
            .state::<Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
        let verified = has_verified_email(db, user.id)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        if !verified {
            return Err(DjangorsError::api(
                StatusCode::FORBIDDEN,
                "email_not_verified",
                "Email address must be verified",
            ));
        }
        Ok(HasVerifiedEmail(user))
    }
}
