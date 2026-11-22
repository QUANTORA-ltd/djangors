//! Turnkey HTML authentication views (`login_view`, `logout_view`,
//! `password_change_view`, `password_reset_view`) and mountable [`auth_routes`].

use std::collections::HashMap;
use std::sync::Arc;

use djangors_core::extract::FromRequest;
use djangors_core::{
    html_escape, DjangorsError, PathParams, Request, Response, Router, StatusCode,
};
use djangors_db::Database;
use djangors_mail::MailBackend;
use djangors_sessions::Session;

use crate::{
    change_password, login, logout, request_password_reset, Auth, AuthBackend, AuthError, AuthUser,
    ModelBackend, PasswordHasherRegistry, User,
};

/// Configuration for the turnkey HTML authentication views mounted by [`auth_routes`].
#[derive(Debug, Clone)]
pub struct AuthViewsConfig {
    /// Route path for the login view (default `"/accounts/login/"`).
    pub login_path: String,
    /// Route path for the logout view (default `"/accounts/logout/"`).
    pub logout_path: String,
    /// Route path for the password change view (default `"/accounts/password_change/"`).
    pub password_change_path: String,
    /// Route path for the password reset view (default `"/accounts/password_reset/"`).
    pub password_reset_path: String,
    /// Default redirect destination after a successful login (default `"/"`).
    pub login_redirect_url: String,
    /// Redirect destination after logout (default `"/"`).
    pub logout_redirect_url: String,
    /// Display name of the site shown in rendered HTML forms (default `"Djangors"`).
    pub site_name: String,
    /// Base URL prefix for password reset links (default `"/accounts/reset/"`).
    pub reset_link_base: String,
}

impl Default for AuthViewsConfig {
    fn default() -> Self {
        Self {
            login_path: "/accounts/login/".to_string(),
            logout_path: "/accounts/logout/".to_string(),
            password_change_path: "/accounts/password_change/".to_string(),
            password_reset_path: "/accounts/password_reset/".to_string(),
            login_redirect_url: "/".to_string(),
            logout_redirect_url: "/".to_string(),
            site_name: "Djangors".to_string(),
            reset_link_base: "/accounts/reset/".to_string(),
        }
    }
}

impl AuthViewsConfig {
    /// Creates an [`AuthViewsConfig`] with default paths and settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Overrides the login redirect destination.
    pub fn login_redirect_url(mut self, url: impl Into<String>) -> Self {
        self.login_redirect_url = url.into();
        self
    }

    /// Overrides the logout redirect destination.
    pub fn logout_redirect_url(mut self, url: impl Into<String>) -> Self {
        self.logout_redirect_url = url.into();
        self
    }

    /// Overrides the site name displayed in HTML templates.
    pub fn site_name(mut self, name: impl Into<String>) -> Self {
        self.site_name = name.into();
        self
    }
}

fn csrf_input(req: &Request) -> String {
    let token = req
        .ext::<djangors_core::middleware::CsrfToken>()
        .map(|t| t.0.clone())
        .or_else(|| {
            let header = req.header("cookie")?.to_str().ok()?;
            header
                .split(';')
                .filter_map(|p| p.trim().split_once('='))
                .find(|(k, _)| *k == "csrftoken")
                .map(|(_, v)| v.to_owned())
        });
    match token {
        Some(t) if !t.is_empty() => format!(
            "<input type=\"hidden\" name=\"csrfmiddlewaretoken\" value=\"{}\">",
            html_escape(&t)
        ),
        _ => String::new(),
    }
}

fn is_safe_redirect_url(url: &str) -> bool {
    url.starts_with('/') && !url.starts_with("//") && !url.contains('\r') && !url.contains('\n')
}

async fn parse_form_fields(req: &Request) -> HashMap<String, String> {
    let body = req.body_bytes().await;
    serde_urlencoded::from_bytes::<Vec<(String, String)>>(body)
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn render_page(title: &str, site_name: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>{title} — {site}</title></head><body>\
         <main><h1>{title}</h1>{body}</main></body></html>",
        title = html_escape(title),
        site = html_escape(site_name),
        body = body,
    )
}

/// Turnkey HTML login view (`GET` renders form, `POST` authenticates and logs in).
pub async fn login_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let config = req.state::<AuthViewsConfig>().cloned().unwrap_or_default();

    if req.method().as_str() == "GET" {
        let next = req.query("next").unwrap_or("");
        let csrf = csrf_input(&req);
        let next_field = if !next.is_empty() {
            format!(
                "<input type=\"hidden\" name=\"next\" value=\"{}\">",
                html_escape(next)
            )
        } else {
            String::new()
        };
        let form = format!(
            "<form method=\"post\">{csrf}{next_field}\
             <label>Username <input type=\"text\" name=\"username\" required></label>\
             <label>Password <input type=\"password\" name=\"password\" required></label>\
             <button type=\"submit\">Log in</button></form>"
        );
        return Ok(Response::html(
            StatusCode::OK,
            render_page("Log in", &config.site_name, &form),
        ));
    }

    let fields = parse_form_fields(&req).await;
    let username = fields.get("username").map(String::as_str).unwrap_or("");
    let password = fields.get("password").map(String::as_str).unwrap_or("");
    let next = fields
        .get("next")
        .map(String::as_str)
        .or_else(|| req.query("next"))
        .unwrap_or("");

    let db = req.require_state::<Database>()?;
    let user_opt = ModelBackend.authenticate(db, username, password).await?;

    if let Some(user) = user_opt {
        let session = req.ext::<Session>().ok_or_else(|| {
            DjangorsError::Internal("session middleware is required for login_view".to_string())
        })?;
        login(session, &user);
        let dest = if is_safe_redirect_url(next) {
            next
        } else {
            &config.login_redirect_url
        };
        return Ok(Response::redirect(dest));
    }

    let csrf = csrf_input(&req);
    let body = format!(
        "<p role=\"alert\">Invalid username or password.</p>\
         <form method=\"post\">{csrf}\
         <label>Username <input type=\"text\" name=\"username\" value=\"{}\" required></label>\
         <label>Password <input type=\"password\" name=\"password\" required></label>\
         <button type=\"submit\">Log in</button></form>",
        html_escape(username)
    );
    Ok(Response::html(
        StatusCode::UNAUTHORIZED,
        render_page("Log in", &config.site_name, &body),
    ))
}

/// Turnkey HTML logout view: clears the session and redirects to `logout_redirect_url`.
pub async fn logout_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let config = req.state::<AuthViewsConfig>().cloned().unwrap_or_default();
    if let Some(session) = req.ext::<Session>() {
        logout(session).await;
    }
    Ok(Response::redirect(&config.logout_redirect_url))
}

/// Turnkey HTML password change view for authenticated users.
pub async fn password_change_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let config = req.state::<AuthViewsConfig>().cloned().unwrap_or_default();
    let Auth(mut user) = Auth::<User>::from_request(&req).await?;

    if req.method().as_str() == "GET" {
        let csrf = csrf_input(&req);
        let form = format!(
            "<form method=\"post\">{csrf}\
             <label>Current password <input type=\"password\" name=\"old_password\" required></label>\
             <label>New password <input type=\"password\" name=\"new_password1\" required></label>\
             <label>Confirm new password <input type=\"password\" name=\"new_password2\" required></label>\
             <button type=\"submit\">Change password</button></form>"
        );
        return Ok(Response::html(
            StatusCode::OK,
            render_page("Change password", &config.site_name, &form),
        ));
    }

    let fields = parse_form_fields(&req).await;
    let old_password = fields.get("old_password").map(String::as_str).unwrap_or("");
    let new_pw1 = fields
        .get("new_password1")
        .map(String::as_str)
        .unwrap_or("");
    let new_pw2 = fields
        .get("new_password2")
        .map(String::as_str)
        .unwrap_or("");

    let render_err = |msg: &str| {
        let csrf = csrf_input(&req);
        let body = format!(
            "<p role=\"alert\">{}</p>\
             <form method=\"post\">{csrf}\
             <label>Current password <input type=\"password\" name=\"old_password\" required></label>\
             <label>New password <input type=\"password\" name=\"new_password1\" required></label>\
             <label>Confirm new password <input type=\"password\" name=\"new_password2\" required></label>\
             <button type=\"submit\">Change password</button></form>",
            html_escape(msg)
        );
        Response::html(
            StatusCode::BAD_REQUEST,
            render_page("Change password", &config.site_name, &body),
        )
    };

    let registry = PasswordHasherRegistry::default();
    if !registry
        .verify(old_password, user.password_hash())
        .unwrap_or(false)
    {
        return Ok(render_err("Current password is incorrect."));
    }
    if new_pw1 != new_pw2 {
        return Ok(render_err("New passwords do not match."));
    }

    match change_password(&req, &mut user, new_pw1).await {
        Ok(()) => Ok(Response::html(
            StatusCode::OK,
            render_page(
                "Password changed",
                &config.site_name,
                "<p>Your password has been changed successfully.</p>",
            ),
        )),
        Err(AuthError::WeakPassword(errs)) => Ok(render_err(&errs.join("; "))),
        Err(e) => Err(e.into()),
    }
}

/// Turnkey HTML password reset request view.
///
/// Always displays a generic confirmation message on `POST` to avoid leaking
/// whether an email address is registered.
pub async fn password_reset_view(
    req: Request,
    _params: PathParams,
) -> Result<Response, DjangorsError> {
    let config = req.state::<AuthViewsConfig>().cloned().unwrap_or_default();

    if req.method().as_str() == "GET" {
        let csrf = csrf_input(&req);
        let form = format!(
            "<form method=\"post\">{csrf}\
             <label>Email <input type=\"email\" name=\"email\" required></label>\
             <button type=\"submit\">Reset password</button></form>"
        );
        return Ok(Response::html(
            StatusCode::OK,
            render_page("Reset password", &config.site_name, &form),
        ));
    }

    let fields = parse_form_fields(&req).await;
    let email = fields.get("email").map(String::as_str).unwrap_or("").trim();

    if !email.is_empty() {
        if let (Some(db), Some(mail)) =
            (req.state::<Database>(), req.state::<Arc<dyn MailBackend>>())
        {
            let secret = req
                .state::<djangors_core::DjangorsSettings>()
                .map(|s| s.secret_key.as_bytes().to_vec())
                .unwrap_or_default();
            let _ = request_password_reset::<User>(
                db,
                mail.as_ref(),
                email,
                &secret,
                &config.reset_link_base,
            )
            .await;
        }
    }

    Ok(Response::html(
        StatusCode::OK,
        render_page(
            "Password reset sent",
            &config.site_name,
            "<p>If an account with that email exists, a password reset link has been sent.</p>",
        ),
    ))
}

/// Builds a mountable [`Router`] registering all turnkey HTML authentication views
/// according to `config`.
pub fn auth_routes(config: AuthViewsConfig) -> Router {
    let login_path = config.login_path.clone();
    let logout_path = config.logout_path.clone();
    let pw_change_path = config.password_change_path.clone();
    let pw_reset_path = config.password_reset_path.clone();

    Router::new()
        .get(&login_path, login_view)
        .post(&login_path, login_view)
        .get(&logout_path, logout_view)
        .post(&logout_path, logout_view)
        .get(&pw_change_path, password_change_view)
        .post(&pw_change_path, password_change_view)
        .get(&pw_reset_path, password_reset_view)
        .post(&pw_reset_path, password_reset_view)
        .with_state(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashers::{PasswordHasher as _, Pbkdf2Sha256Hasher};
    use djangors_orm::Model as _;
    use djangors_sessions::Session;
    use djangors_test::{TestClient, TestDatabase};

    async fn setup_auth_tables(db: &TestDatabase) {
        let pk = db.database().dialect().auto_pk_type();
        let ts = db.database().dialect().timestamp_type();
        let bigint = db.database().dialect().bigint_type();
        db.create_table(&format!(
            "CREATE TABLE IF NOT EXISTS auth_user (
                id {pk},
                username VARCHAR(150) NOT NULL UNIQUE,
                email VARCHAR(254) NOT NULL,
                password TEXT NOT NULL,
                is_active BOOLEAN NOT NULL,
                is_staff BOOLEAN NOT NULL,
                is_superuser BOOLEAN NOT NULL,
                date_joined {ts} NOT NULL,
                last_login {ts}
            )"
        ))
        .await
        .unwrap();
        db.create_table(&format!(
            "CREATE TABLE IF NOT EXISTS auth_session_generation (
                id {pk},
                user_id {bigint} NOT NULL UNIQUE,
                generation {bigint} NOT NULL
            )"
        ))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn auth_views_login_logout_and_transparent_rehash() {
        let db = TestDatabase::isolated().await.unwrap();
        setup_auth_tables(&db).await;

        let legacy_hash = Pbkdf2Sha256Hasher::new(1_000)
            .encode("S3cur3P@ssword!")
            .unwrap();
        User {
            id: 0,
            username: "alice".to_string(),
            email: "alice@example.com".to_string(),
            password: legacy_hash,
            is_active: true,
            is_staff: false,
            is_superuser: false,
            date_joined: chrono::Utc::now(),
            last_login: None,
        }
        .save(db.database())
        .await
        .unwrap();

        let router = auth_routes(AuthViewsConfig::default().login_redirect_url("/dashboard"))
            .with_state(db.database().clone());
        let client = TestClient::new(router);

        // GET login form
        client
            .get("/accounts/login/")
            .send()
            .await
            .assert_status(StatusCode::OK)
            .assert_contains("<form method=\"post\">");

        // POST login with legacy PBKDF2 password -> succeeds and transparently upgrades to Argon2id
        let session = Session::new_empty();
        let resp = client
            .post_form(
                "/accounts/login/",
                &[("username", "alice"), ("password", "S3cur3P@ssword!")],
            )
            .with_state(db.database().clone())
            .with_session(session.clone())
            .send()
            .await;
        assert_eq!(
            resp.status(),
            StatusCode::FOUND,
            "body: {}",
            resp.body_str()
        );
        assert!(session.get::<i64>(crate::SESSION_USER_ID_KEY).is_some());

        // Verify password hash in DB was upgraded to Argon2id
        let upgraded = User::objects()
            .filter(djangors_orm::q!(username = "alice"))
            .unwrap()
            .get(db.database())
            .await
            .unwrap();
        assert!(
            upgraded.password.starts_with("$argon2id$"),
            "expected upgraded Argon2id hash, got {}",
            upgraded.password
        );

        // POST logout clears session
        let logout_resp = client
            .post_form("/accounts/logout/", &[])
            .with_session(session.clone())
            .send()
            .await;
        assert_eq!(logout_resp.status(), StatusCode::FOUND);
        assert!(session.get::<i64>(crate::SESSION_USER_ID_KEY).is_none());

        // GET & POST password reset
        client
            .get("/accounts/password_reset/")
            .send()
            .await
            .assert_status(StatusCode::OK)
            .assert_contains("name=\"email\"");
        client
            .post_form(
                "/accounts/password_reset/",
                &[("email", "alice@example.com")],
            )
            .send()
            .await
            .assert_status(StatusCode::OK)
            .assert_contains("password reset link");

        db.cleanup().await.unwrap();
    }
}
