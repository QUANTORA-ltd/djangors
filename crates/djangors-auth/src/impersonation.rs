//! Admin impersonation ("log in as user").
//!
//! A superuser starts impersonation with `POST /admin/impersonate/{user_id}`. The session then
//! belongs to the target user (`Auth<User>` returns the target) and additionally records the
//! real actor under [`IMPERSONATOR_KEY`]. `POST /admin/impersonate/stop` restores the
//! superuser's session.
//!
//! Guard rails:
//! - Only active superusers can start impersonation.
//! - Superusers cannot be impersonated, and nobody can impersonate themselves.
//! - Impersonation cannot be nested. While impersonating, [`ensure_not_impersonating`] (or the
//!   [`NotImpersonating`] extractor) refuses sensitive actions such as password, email and
//!   API key changes.
//! - Every start and stop is written to `auth_impersonation_log`.

use crate::session_mgmt::{get_session_generation, SESSION_GENERATION_KEY};
use crate::{Auth, AuthError, User, SESSION_USER_ID_KEY};
use chrono::Utc;
use djangors_core::extract::FromRequest;
use djangors_core::{DjangorsError, PathParams, Request, Response, Router, StatusCode};
use djangors_db::{BindValue, Database};
use djangors_orm::Model as _;
use djangors_sessions::Session;

/// Session key holding the real (superuser) user id while impersonating.
pub const IMPERSONATOR_KEY: &str = "_auth_impersonator_id";

/// Creates the `auth_impersonation_log` table. Run once at startup or in a migration.
pub async fn create_impersonation_log_table(db: &Database) -> Result<(), AuthError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let bigint = dialect.bigint_type();
    let ts = dialect.timestamp_type();
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS auth_impersonation_log (
            id {pk},
            action VARCHAR(16) NOT NULL,
            actor_id {bigint} NOT NULL,
            target_id {bigint} NOT NULL,
            created_at {ts} NOT NULL
        )"
    );
    conn.execute(&sql, &[])
        .await
        .map_err(|e| AuthError::Database(djangors_orm::OrmError::Query(e)))?;
    Ok(())
}

/// Reads the impersonator id from the session on `req`, if impersonation is active.
pub fn impersonator_id(req: &Request) -> Option<i64> {
    req.ext::<Session>()?.get::<i64>(IMPERSONATOR_KEY)
}

/// Returns `true` while the session on `req` is impersonating another user.
pub fn is_impersonating(req: &Request) -> bool {
    impersonator_id(req).is_some()
}

/// Returns `Err(Forbidden)` while impersonating. Call this before any sensitive action
/// (password or email change, API key creation, starting another impersonation).
pub fn ensure_not_impersonating(req: &Request) -> Result<(), DjangorsError> {
    if is_impersonating(req) {
        return Err(DjangorsError::api(
            StatusCode::FORBIDDEN,
            "impersonation_restricted",
            "This action is not allowed while impersonating another user",
        ));
    }
    Ok(())
}

/// Extractor form of [`ensure_not_impersonating`], for handler arguments that should refuse
/// impersonated sessions.
#[derive(Debug, Clone, Copy)]
pub struct NotImpersonating;

#[async_trait::async_trait]
impl FromRequest for NotImpersonating {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        ensure_not_impersonating(req)?;
        Ok(NotImpersonating)
    }
}

/// Extension trait that adds impersonation accessors to [`Request`].
///
/// It is a trait because `Request` lives in `djangors-core`, which cannot name [`User`].
#[async_trait::async_trait]
pub trait RequestImpersonation {
    /// Real (superuser) user id while impersonating, otherwise `None`.
    fn impersonator_id(&self) -> Option<i64>;
    /// The real user behind an impersonated session, or `None` when not impersonating.
    async fn impersonator(&self) -> Result<Option<User>, DjangorsError>;
}

#[async_trait::async_trait]
impl RequestImpersonation for Request {
    fn impersonator_id(&self) -> Option<i64> {
        impersonator_id(self)
    }

    async fn impersonator(&self) -> Result<Option<User>, DjangorsError> {
        let Some(id) = impersonator_id(self) else {
            return Ok(None);
        };
        let db = self
            .state::<Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
        let users = User::objects()
            .filter(djangors_orm::q!(id = id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?;
        Ok(users.into_iter().next())
    }
}

/// Plain-HTML banner shown while impersonating. Empty when the session is not impersonating.
/// Templates should render it near the top of every page.
pub fn impersonation_banner_html(req: &Request) -> String {
    if !is_impersonating(req) {
        return String::new();
    }
    let csrf_field = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| {
            format!(
                "<input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{}\">",
                djangors_core::html_escape(&t.0)
            )
        })
        .unwrap_or_default();
    format!(
        "<div class=\"djangors-impersonation-banner\" role=\"alert\">\
         You are impersonating another user. \
         <form method=\"post\" action=\"/admin/impersonate/stop\" style=\"display:inline\">{csrf_field}\
         <button type=\"submit\">Stop impersonating</button></form></div>"
    )
}

async fn record_audit(
    db: &Database,
    action: &str,
    actor_id: i64,
    target_id: i64,
) -> Result<(), DjangorsError> {
    let mut conn = db.conn();
    let p1 = conn.dialect().placeholder(1);
    let p2 = conn.dialect().placeholder(2);
    let p3 = conn.dialect().placeholder(3);
    let p4 = conn.dialect().placeholder(4);
    conn.execute(
        &format!(
            "INSERT INTO auth_impersonation_log (action, actor_id, target_id, created_at)
             VALUES ({p1}, {p2}, {p3}, {p4})"
        ),
        &[
            BindValue::Text(action.to_owned()),
            BindValue::I64(actor_id),
            BindValue::I64(target_id),
            BindValue::DateTime(Utc::now()),
        ],
    )
    .await
    .map_err(|e| DjangorsError::Internal(format!("impersonation audit failed: {e}")))?;
    Ok(())
}

fn session_of(req: &Request) -> Result<&Session, DjangorsError> {
    req.ext::<Session>()
        .ok_or_else(|| DjangorsError::Unauthorized("session extension absent".to_string()))
}

fn db_of(req: &Request) -> Result<&Database, DjangorsError> {
    req.state::<Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))
}

/// `POST /admin/impersonate/{user_id}`: starts impersonating `user_id`.
pub async fn start_impersonation_view(
    req: Request,
    params: PathParams,
) -> Result<Response, DjangorsError> {
    ensure_not_impersonating(&req)?;
    let target_id: i64 = params
        .get("user_id")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| DjangorsError::BadRequest("invalid user id".to_string()))?;
    let Auth(actor) = <Auth<User> as FromRequest>::from_request(&req).await?;
    if !actor.is_superuser {
        return Err(DjangorsError::Forbidden(
            "superuser access required".to_string(),
        ));
    }
    if actor.id == target_id {
        return Err(DjangorsError::BadRequest(
            "cannot impersonate yourself".to_string(),
        ));
    }
    let db = db_of(&req)?;
    let target = User::objects()
        .filter(djangors_orm::q!(id = target_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .into_iter()
        .next()
        .ok_or(DjangorsError::NotFound)?;
    if target.is_superuser {
        return Err(DjangorsError::Forbidden(
            "superusers cannot be impersonated".to_string(),
        ));
    }
    if !target.is_active {
        return Err(DjangorsError::BadRequest(
            "target account is inactive".to_string(),
        ));
    }

    let target_gen = get_session_generation(db, target.id)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;
    let session = session_of(&req)?;
    session.cycle_key();
    session.set(IMPERSONATOR_KEY, actor.id);
    session.set(SESSION_USER_ID_KEY, target.id);
    session.set(SESSION_GENERATION_KEY, target_gen);

    record_audit(db, "start", actor.id, target.id).await?;
    Ok(Response::redirect("/"))
}

/// `POST /admin/impersonate/stop`: ends impersonation and restores the superuser's session.
pub async fn stop_impersonation_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let actor_id = impersonator_id(&req)
        .ok_or_else(|| DjangorsError::BadRequest("not impersonating".to_string()))?;
    let session = session_of(&req)?;
    let target_id = session
        .get::<i64>(SESSION_USER_ID_KEY)
        .ok_or_else(|| DjangorsError::Unauthorized("not authenticated".to_string()))?;
    let db = db_of(&req)?;

    // The real account must still exist, be active and be a superuser. Otherwise the
    // session is cleared rather than restored, so a revoked superuser cannot keep access.
    let actor = User::objects()
        .filter(djangors_orm::q!(id = actor_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .into_iter()
        .next();

    session.remove(IMPERSONATOR_KEY);
    session.cycle_key();
    match actor {
        Some(actor) if actor.is_active && actor.is_superuser => {
            let gen = get_session_generation(db, actor.id)
                .await
                .map_err(|e| DjangorsError::Internal(e.to_string()))?;
            session.set(SESSION_USER_ID_KEY, actor.id);
            session.set(SESSION_GENERATION_KEY, gen);
        }
        _ => {
            session.clear();
            record_audit(db, "stop_revoked", actor_id, target_id).await?;
            return Err(DjangorsError::Forbidden(
                "impersonating superuser no longer has access; session cleared".to_string(),
            ));
        }
    }

    record_audit(db, "stop", actor_id, target_id).await?;
    Ok(Response::redirect("/"))
}

/// Mountable router for `POST /admin/impersonate/{user_id}` and `POST /admin/impersonate/stop`.
pub fn impersonation_router() -> Router {
    Router::new()
        .post("/admin/impersonate/{user_id:i64}", start_impersonation_view)
        .post("/admin/impersonate/stop", stop_impersonation_view)
}

#[cfg(test)]
#[path = "impersonation_tests.rs"]
mod impersonation_tests;
