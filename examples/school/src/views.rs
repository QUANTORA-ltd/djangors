use djangors_auth::AuthBackend;
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response};

#[derive(serde::Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
}

pub async fn login_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let session = req.ext::<djangors_sessions::Session>().ok_or_else(|| {
        DjangorsError::Internal("session extension absent (SessionLayer misconfigured)".to_string())
    })?;

    let Form(form) = Form::<LoginForm>::from_request(&req).await?;

    let backend = djangors_auth::ModelBackend;
    match backend
        .authenticate(db, &form.username, &form.password)
        .await
    {
        Ok(Some(user)) => {
            djangors_auth::login(session, &user);
            Ok(Response::redirect("/"))
        }
        Ok(None) => Ok(Response::redirect("/accounts/login/?error=1")),
        Err(e) => Err(DjangorsError::Internal(e.to_string())),
    }
}

pub async fn logout_view(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let session = req.ext::<djangors_sessions::Session>().ok_or_else(|| {
        DjangorsError::Internal("session extension absent (SessionLayer misconfigured)".to_string())
    })?;

    djangors_auth::logout(session).await;
    Ok(Response::redirect("/"))
}
