use djangors_auth::{Auth, AuthBackend};
use djangors_core::extract::{Form, FromRequest};
use djangors_core::{DjangorsError, PathParams, Request, Response, StatusCode};
use djangors_orm::{q, Model};

use crate::models::{Choice, Question};

pub async fn hello(_req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    Ok(Response::text(StatusCode::OK, "Hello, world!"))
}

pub async fn index(req: Request, _params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let latest_question_list = Question::objects()
        .filter(q!(pub_date__lte = chrono::Utc::now()))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .order_by("-pub_date")
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .limit(5)
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let body = latest_question_list
        .iter()
        .map(|q| format!("<li><a href=\"/{}/\">{}</a></li>", q.id, q.question_text))
        .collect::<String>();
    Ok(Response::html(StatusCode::OK, format!("<ul>{body}</ul>")))
}

pub async fn detail(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let question_id: i64 = params.get_as("question_id")?;

    let question = Question::objects()
        .filter(q!(id = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })?;

    let choices = Choice::objects()
        .filter(q!(question = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let choices_html = choices
        .iter()
        .map(|c| {
            format!(
                r#"<li>
                    <input type="radio" name="choice" id="choice_{}" value="{}">
                    <label for="choice_{}">{}</label>
                </li>"#,
                c.id, c.id, c.id, c.choice_text
            )
        })
        .collect::<String>();

    let html = format!(
        r#"<h1>{}</h1>
        <form action="/{}/vote/" method="post">
            <ul>{}</ul>
            <input type="submit" value="Vote">
        </form>"#,
        question.question_text, question.id, choices_html
    );

    Ok(Response::html(StatusCode::OK, html))
}

pub async fn vote(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    // login-gating requirement: extract AuthUser via the manual pattern
    let _auth = Auth::<djangors_auth::User>::from_request(&req).await?;

    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

    let question_id: i64 = params.get_as("question_id")?;

    #[derive(serde::Deserialize)]
    struct VoteForm {
        choice: i64,
    }
    let Form(vote) = Form::<VoteForm>::from_request(&req).await?;

    let question = Question::objects()
        .filter(q!(id = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })?;

    Choice::objects()
        .filter(q!(question = question.id, id = vote.choice))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .update(
            db,
            djangors_orm::set!(votes = djangors_orm::F("votes") + 1i64),
        )
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    // Named-route reversal doesn't exist yet, so we use a hardcoded format string path as a stand-in per design 4.16
    Ok(Response::redirect(&format!("/{}/results/", question_id)))
}

pub async fn results(req: Request, params: PathParams) -> Result<Response, DjangorsError> {
    let db = req
        .state::<djangors_db::Database>()
        .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;
    let question_id: i64 = params.get_as("question_id")?;

    let question = Question::objects()
        .filter(q!(id = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .get(db)
        .await
        .map_err(|e| match e {
            djangors_orm::OrmError::NotFound { .. } => DjangorsError::NotFound,
            _ => DjangorsError::Internal(e.to_string()),
        })?;

    let choices = Choice::objects()
        .filter(q!(question = question_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?;

    let choices_html = choices
        .iter()
        .map(|c| format!("<li>{} -- {} vote(s)</li>", c.choice_text, c.votes))
        .collect::<String>();

    Ok(Response::html(
        StatusCode::OK,
        format!(
            "<h1>Results for {}</h1><ul>{}</ul><a href=\"/{}/\">Vote again?</a>",
            question.question_text, choices_html, question.id
        ),
    ))
}

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
